use super::{FlowEnd, FlowGraph, Stmt};
use crate::index::{BlockId, IdVec, LocalId, ValueId};
use crate::ir::{MethodIr, Statement, Terminator, ValueUse};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn recover(
    analysis: &super::finally::Analysis<'_>,
    proof: MonitorProof,
    filter: Option<&super::HandlerFilter<'_>>,
) -> Option<super::cleanup::Recovery> {
    let ir = analysis.ir();
    let all_monitors = proof.operations;
    let mut regions = super::regions::RegionGraph::new(ir);
    let mut suppressed = BTreeSet::new();
    let mut enters = Vec::new();
    for (&(block, index), &depth) in &proof.enter_depths {
        if let Statement::Monitor { value, enter: true } = ir.blocks[block].statements[index] {
            enters.push((depth, block, index, value));
        }
    }
    // Appended source-preparation blocks do not retain bytecode layout order.
    // Suppress nested monitors first using the already-proved ownership stack.
    enters.sort_by_key(|&(depth, block, index, _)| {
        (
            std::cmp::Reverse(depth),
            std::cmp::Reverse(ir.blocks[block].pc),
            block,
            index,
        )
    });
    for (_, entry, index, monitor) in enters {
        let graph = &regions.flow;
        if index + 1 != ir.blocks[entry].statements.len() {
            return None;
        }
        let FlowEnd::Goto(start) = graph.blocks[entry].end else {
            return None;
        };
        let (handler, cleanup) = find_cleanup(ir, graph, start, monitor, &suppressed)
            .unwrap_or((BlockId::from_index(usize::MAX), Vec::new()));
        // Optimizers may remove entries shadowed by the monitor catch-all.
        // Its release/rethrow continuation determines the enclosing dispatch.
        let outer = cleanup.last().map_or_else(Vec::new, |&tail| {
            effective_handlers(&graph.blocks[tail].handlers)
        });
        let same = |a: &crate::cfg::HandlerEdge, b: &crate::cfg::HandlerEdge| {
            a.target == b.target && a.catch_type == b.catch_type
        };
        let entry_handlers = effective_handlers(&graph.blocks[entry].handlers);
        let canonical_cleanup = !cleanup.is_empty()
            && entry_handlers.len() == outer.len()
            && entry_handlers.iter().zip(&outer).all(|(a, b)| same(a, b));
        let mut selected: IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
        let mut pending = vec![start];
        let mut exits = BTreeSet::new();
        let mut release_edges = Vec::new();
        let mut monitors = Vec::new();
        while let Some(node) = pending.pop() {
            if selected[node] {
                continue;
            }
            if canonical_cleanup && cleanup.contains(&node) {
                return None;
            }
            selected[node] = true;
            let mut released = false;
            for (index, statement) in ir.blocks[node].statements.iter().enumerate() {
                if suppressed.contains(&(node, index)) {
                    continue;
                }
                if let Statement::Monitor { value, enter } = statement {
                    if *value != monitor
                        || *enter
                        || released
                        || index + 1 != ir.blocks[node].statements.len()
                    {
                        return None;
                    }
                    released = true;
                    monitors.push((node, index));
                }
            }
            if !released {
                if matches!(graph.blocks[node].end, FlowEnd::Return) {
                    return None;
                }
                pending.extend(graph.blocks[node].end.successors());
            } else {
                for target in graph.blocks[node].end.successors() {
                    release_edges.push(target);
                }
            }
            if body_may_throw(ir, node) {
                pending.extend(
                    effective_handlers(&graph.blocks[node].handlers)
                        .iter()
                        .filter(|edge| {
                            !canonical_cleanup
                                || (edge.target != handler
                                    && !outer.iter().any(|other| same(edge, other)))
                        })
                        .map(|edge| edge.target),
                );
            }
        }
        for target in release_edges {
            let tail = pure_return_tail(ir, target, &all_monitors).filter(|tail| {
                // A plain return is a source continuation, not part of the
                // monitor. Keep it visible to enclosing cleanup proofs. Only
                // nested release chains need to travel with the pending exit.
                tail.iter().any(|&node| {
                    ir.blocks[node].statements.iter().any(|statement| {
                        matches!(statement, Statement::Monitor { enter: false, .. })
                    })
                }) && graph.blocks.iter_enumerated().all(|(source, block)| {
                    selected[source]
                        || tail.contains(&source)
                        || !block
                            .end
                            .successors()
                            .iter()
                            .any(|target| tail.contains(target))
                })
            });
            if let Some(tail) = tail {
                for node in tail {
                    selected[node] = true;
                }
            } else {
                exits.insert(target);
            }
        }
        let multiple_exits = exits.len() > 1;
        let follow = exits.first().copied();
        for (source, block) in graph.blocks.iter_enumerated() {
            if selected[source] || source == entry {
                continue;
            }
            if block
                .end
                .successors()
                .iter()
                .any(|&target| selected[target])
            {
                return None;
            }
        }
        let mut sub = graph.clone();
        for (node, block) in sub.blocks.iter_enumerated_mut() {
            if selected[node] {
                // Remove unreachable table entries before subtracting the
                // lexical scope. Removing its catch-all must not expose an
                // entry that the original JVM dispatch could never reach.
                block.handlers = effective_handlers(&block.handlers);
                if canonical_cleanup {
                    block.handlers.retain(|edge| {
                        edge.target != handler && !outer.iter().any(|other| same(edge, other))
                    });
                }
                block.may_throw = body_may_throw(ir, node);
            }
        }
        // Other handlers within the monitor remain real catches. Their cleanup
        // edge belongs to this synchronized region and is removed recursively.
        if canonical_cleanup {
            for &node in &cleanup {
                sub.blocks[node]
                    .handlers
                    .retain(|edge| edge.target != handler);
            }
        }
        // Cleanup proofs inside a monitor must precede lexical handler
        // lowering too. Its already-proved normal exits are opaque boundaries:
        // no cleanup candidate may inspect or absorb code beyond the monitor.
        sub.entry = start;
        let mut replacements = regions.replacements.clone();
        for (node, block) in sub.blocks.iter_enumerated_mut() {
            if !selected[node] {
                super::installation::tombstone(block);
            }
        }
        for &target in &exits {
            replacements.insert(target, vec![Stmt::Jump(target)]);
        }
        let mut nested = super::regions::RegionGraph {
            flow: sub,
            replacements,
            cleanups: Default::default(),
        };
        nested.recover_cleanups(ir, |regions, candidate| {
            super::finally::plan_candidate(analysis, regions, candidate, filter)
        });
        let nested_cleanups = nested.cleanups.clone();
        let mut body = nested.lower(filter).ok()?.body;
        if !multiple_exits && let Some(target) = follow {
            body = super::bindings::bind(body, &[(target, Vec::new())]);
        }
        // Exceptional SSA copies in the release/rethrow path are real local
        // updates. Preserve them in a catch inside the synchronized scope;
        // these nonthrowing copies commute with its implicit monitor release.
        if canonical_cleanup
            && cleanup.iter().any(|&node| {
                ir.blocks[node]
                    .statements
                    .iter()
                    .any(|statement| !matches!(statement, Statement::Monitor { .. }))
            })
        {
            let mut catch_body: Vec<_> = cleanup.iter().copied().map(Stmt::Basic).collect();
            catch_body.push(Stmt::Throw(*cleanup.last()?));
            body = vec![Stmt::Try {
                body,
                catches: vec![super::Catch {
                    catch_type: None,
                    handler,
                    body: catch_body,
                }],
            }];
        }
        regions.install(super::regions::VerifiedRegion {
            entry,
            nodes: std::iter::once(entry)
                .chain(
                    selected
                        .iter_enumerated()
                        .filter_map(|(node, &yes)| yes.then_some(node)),
                )
                .collect(),
            // Acquisition runs before the synchronized body and keeps its
            // original protection and throwing policy.
            outer: graph.blocks[entry].handlers.clone(),
            may_throw: graph.blocks[entry].may_throw,
            end: if multiple_exits {
                FlowEnd::Multi(exits.into_iter().map(|target| (target, target)).collect())
            } else {
                follow.map_or(FlowEnd::Unreachable, FlowEnd::Goto)
            },
            body: vec![Stmt::Basic(entry), Stmt::Synchronized { monitor, body }],
            cleanups: nested_cleanups,
        });
        suppressed.insert((entry, index));
        suppressed.extend(monitors);
        for &node in &cleanup {
            for (index, statement) in ir.blocks[node].statements.iter().enumerate() {
                if matches!(statement, Statement::Monitor { .. }) {
                    suppressed.insert((node, index));
                }
            }
        }
    }
    regions.recover_cleanups(ir, |regions, candidate| {
        super::finally::plan_candidate(analysis, regions, candidate, filter)
    });
    Some(super::cleanup::Recovery {
        regions,
        suppressed_monitors: all_monitors.into_iter().collect(),
    })
}

fn body_may_throw(ir: &MethodIr, node: BlockId) -> bool {
    ir.blocks[node].statements.iter().any(|statement| {
        !matches!(statement, Statement::Monitor { enter: false, .. })
            && super::statement_may_throw(ir, statement)
    }) || super::terminator_may_throw(ir, &ir.blocks[node].terminator)
}

fn effective_handlers(handlers: &[crate::cfg::HandlerEdge]) -> Vec<crate::cfg::HandlerEdge> {
    crate::cfg::dispatch::effective(handlers).cloned().collect()
}

fn find_cleanup(
    ir: &MethodIr,
    graph: &FlowGraph,
    start: BlockId,
    monitor: ValueId,
    suppressed: &BTreeSet<(BlockId, usize)>,
) -> Option<(BlockId, Vec<BlockId>)> {
    let mut seen: IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
    let mut pending = vec![start];
    while let Some(node) = pending.pop() {
        if std::mem::replace(&mut seen[node], true) {
            continue;
        }
        let handlers = effective_handlers(&graph.blocks[node].handlers);
        for handler in &handlers {
            if handler
                .catch_type
                .as_deref()
                .is_none_or(|ty| ty == "java/lang/Throwable")
                && let Some(cleanup) = cleanup(ir, handler.target, monitor)
            {
                return Some((handler.target, cleanup));
            }
        }
        let releases = ir.blocks[node].statements.iter().enumerate().any(|(index, statement)| {
            !suppressed.contains(&(node, index)) && matches!(statement, Statement::Monitor { value, enter: false } if *value == monitor)
        });
        if !releases {
            pending.extend(graph.blocks[node].end.successors());
            pending.extend(handlers.iter().map(|handler| handler.target));
        }
    }
    None
}

fn cleanup(ir: &MethodIr, handler: BlockId, monitor: ValueId) -> Option<Vec<BlockId>> {
    let caught = ir.blocks[handler].caught_exception?;
    let mut copies: BTreeSet<_> = ir.values[caught].local.into_iter().collect();
    let mut nodes = Vec::new();
    let mut current = handler;
    let mut released = false;
    loop {
        if nodes.contains(&current) {
            return None;
        }
        nodes.push(current);
        for statement in &ir.blocks[current].statements {
            match statement {
                Statement::Monitor {
                    value,
                    enter: false,
                } if *value == monitor && !released => released = true,
                Statement::Assign { local, value }
                    if !super::statement_may_throw(ir, statement) =>
                {
                    if assigns_local(ir, *value) {
                        return None;
                    }
                    if copies_caught(ir, *value, caught, &copies) {
                        copies.insert(*local);
                    } else {
                        copies.remove(local);
                    }
                }
                _ => return None,
            }
        }
        match ir.blocks[current].terminator {
            Terminator::Goto(next) => current = next,
            Terminator::Throw { value, .. }
                if released && copies_caught(ir, ValueUse::Reference(value), caught, &copies) =>
            {
                return Some(nodes);
            }
            _ => return None,
        }
    }
}

// Replay actual scheduled copies along the cleanup path. A Phi's inputs may
// include another handler (for a failed monitorexit), but only the local value
// established on this path determines the rethrow. Never equate the SSA IDs.
fn copies_caught(
    ir: &MethodIr,
    mut usage: ValueUse,
    caught: ValueId,
    copies: &BTreeSet<LocalId>,
) -> bool {
    let mut seen = BTreeSet::new();
    while seen.insert(usage.value()) {
        if let Some(local) = usage.local(ir) {
            return copies.contains(&local);
        }
        let value = usage.value();
        if value == caught {
            return true;
        }
        if let crate::ir::ValueKind::Alias(source) = ir.values[value].kind {
            usage = ValueUse::Reference(source);
        } else {
            return false;
        }
    }
    false
}

fn assigns_local(ir: &MethodIr, usage: ValueUse) -> bool {
    crate::ir::scheduled::values(ir, usage)
        .operations()
        .any(|value| ir.values[value].kind.local_write().is_some())
}

fn pure_return_tail(
    ir: &MethodIr,
    mut current: BlockId,
    monitors: &BTreeSet<(BlockId, usize)>,
) -> Option<Vec<BlockId>> {
    let mut nodes = Vec::new();
    loop {
        if nodes.contains(&current) {
            return None;
        }
        for (index, statement) in ir.blocks[current].statements.iter().enumerate() {
            if !matches!(statement, Statement::Monitor { enter: false, .. })
                || !monitors.contains(&(current, index))
            {
                return None;
            }
        }
        nodes.push(current);
        match ir.blocks[current].terminator {
            Terminator::Goto(next) => current = next,
            Terminator::Return(value) => {
                if value.is_none_or(|value| {
                    !crate::ir::scheduled::value_may_throw(ir, ValueUse::Reference(value))
                }) {
                    return Some(nodes);
                }
                return None;
            }
            _ => return None,
        }
    }
}

/// Check ownership before moving lexical monitor boundaries. In particular,
/// an early return through nested releases is safe to represent by a return
/// inside nested synchronized statements only when the original release order
/// matches the same monitor stack on every incoming path.
pub(super) struct MonitorProof {
    operations: BTreeSet<(BlockId, usize)>,
    enter_depths: BTreeMap<(BlockId, usize), usize>,
}

impl MonitorProof {
    pub fn cleanups(&self, ir: &MethodIr) -> super::cleanup::CleanupFacts {
        let values: BTreeSet<_> = self
            .enter_depths
            .keys()
            .filter_map(|&(block, index)| match ir.blocks[block].statements[index] {
                Statement::Monitor { value, enter: true } => Some(value),
                _ => None,
            })
            .collect();
        let mut facts = super::cleanup::CleanupFacts::default();
        for handler in ir
            .blocks
            .iter()
            .flat_map(|block| effective_handlers(&block.handlers))
        {
            if !crate::cfg::dispatch::is_catch_all(handler.catch_type.as_deref()) {
                continue;
            }
            for &monitor in &values {
                let Some(nodes) = cleanup(ir, handler.target, monitor) else {
                    continue;
                };
                // Ownership plus this same-object release path proves the
                // implicit operation before choosing any source strategy.
                // Exceptional local copies instead require an explicit catch.
                if nodes.iter().all(|&node| {
                    ir.blocks[node]
                        .statements
                        .iter()
                        .all(|statement| matches!(statement, Statement::Monitor { .. }))
                }) {
                    facts.record(ir, handler.target, nodes.last().copied());
                }
            }
        }
        facts
    }
}

pub(super) fn validate_monitor_stacks(ir: &MethodIr) -> Option<MonitorProof> {
    fn send(
        states: &mut IdVec<BlockId, Option<usize>>,
        pending: &mut Vec<BlockId>,
        target: BlockId,
        state: usize,
    ) -> Option<()> {
        if let Some(previous) = states[target] {
            if previous != state {
                return None;
            }
        } else {
            states[target] = Some(state);
            pending.push(target);
        }
        Some(())
    }
    fn exceptional(
        ir: &MethodIr,
        block: BlockId,
        state: usize,
        states: &mut IdVec<BlockId, Option<usize>>,
        pending: &mut Vec<BlockId>,
    ) -> Option<()> {
        let mut exhaustive = false;
        for handler in &ir.blocks[block].handlers {
            send(states, pending, handler.target, state)?;
            if handler
                .catch_type
                .as_deref()
                .is_none_or(|ty| ty == "java/lang/Throwable")
            {
                exhaustive = true;
                break;
            }
        }
        if state != 0 && !exhaustive {
            return None;
        }
        Some(())
    }
    let mut states: IdVec<BlockId, _> = vec![None; ir.blocks.len()].into();
    let mut pending = Vec::new();
    send(&mut states, &mut pending, ir.entry, 0)?;
    // Intern stack tails: joins compare one ID and deeply nested monitors do
    // not copy an increasingly long vector into every block's entry frame.
    let mut stack_nodes = vec![(0usize, ValueId::from_index(usize::MAX), 0usize)];
    let mut stack_ids = std::collections::HashMap::new();
    let mut monitors = BTreeSet::new();
    let mut enter_depths = BTreeMap::new();
    while let Some(block) = pending.pop() {
        let mut state = states[block]?;
        for (index, statement) in ir.blocks[block].statements.iter().enumerate() {
            match statement {
                Statement::Monitor { value, enter: true } => {
                    exceptional(ir, block, state, &mut states, &mut pending)?;
                    state = *stack_ids.entry((state, *value)).or_insert_with(|| {
                        let id = stack_nodes.len();
                        stack_nodes.push((state, *value, stack_nodes[state].2 + 1));
                        id
                    });
                    monitors.insert((block, index));
                    enter_depths.insert((block, index), stack_nodes[state].2);
                }
                Statement::Monitor {
                    value,
                    enter: false,
                } => {
                    if state == 0 || stack_nodes[state].1 != *value {
                        return None;
                    }
                    state = stack_nodes[state].0;
                    monitors.insert((block, index));
                }
                _ if super::statement_may_throw(ir, statement) => {
                    exceptional(ir, block, state, &mut states, &mut pending)?
                }
                _ => (),
            }
        }
        let terminal = &ir.blocks[block].terminator;
        if super::terminator_may_throw(ir, terminal) {
            exceptional(ir, block, state, &mut states, &mut pending)?;
        }
        if matches!(terminal, Terminator::Return(_)) && state != 0 {
            return None;
        }
        for target in terminal.successors() {
            send(&mut states, &mut pending, target, state)?;
        }
    }
    Some(MonitorProof {
        operations: monitors,
        enter_depths,
    })
}
