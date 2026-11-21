//! Recover lexical try regions from ordered dispatch, keeping catches in table
//! order and leaving duplicated cleanup as ordinary code. In particular a
//! catch-all is not evidence that every normal exit executes a finally body.

use super::analysis::{Dominators, reachability_with_handlers};
use super::attempt::{
    self, RecoveryAttempt, RecoveryRejection, RecoveryResult, RecoveryStrategy, RejectionReason,
};
use super::installation::expand;
use super::{Catch, FlowBlock, FlowEnd, FlowGraph, Stmt};
use crate::cfg::HandlerEdge;
use crate::cfg::dispatch::{effective, same_dispatch as signature, same_handler as same};
use crate::index::{BlockId, IdVec};
use std::collections::{BTreeMap, BTreeSet};

pub(super) mod coverage;

#[cfg(test)]
mod tests;

#[cfg(test)]
fn recover(graph: &FlowGraph) -> Option<Vec<Stmt>> {
    recover_with_eligibility(
        graph,
        &super::eligibility::Eligibility::default(),
        &mut Vec::new(),
    )
}

pub(super) fn recover_with_eligibility(
    graph: &FlowGraph,
    eligibility: &super::eligibility::Eligibility<'_>,
    attempts: &mut Vec<RecoveryAttempt>,
) -> Option<Vec<Stmt>> {
    match recover_inner(graph.clone(), 0, true, eligibility.clone(), attempts) {
        Ok(body) => Some(body),
        Err(rejection) => attempt::record(
            attempts,
            graph.entry,
            RecoveryStrategy::ExceptionRegions,
            Err(rejection),
        )
        .ok(),
    }
}

fn recover_inner(
    mut graph: FlowGraph,
    depth: usize,
    allow_dispatch: bool,
    mut eligibility: super::eligibility::Eligibility<'_>,
    attempts: &mut Vec<RecoveryAttempt>,
) -> RecoveryResult<Vec<Stmt>> {
    if depth > 48 {
        return Err(RecoveryRejection::at(
            graph.entry,
            RejectionReason::NestingLimit,
        ));
    }
    // Keep shadowed entries until their lexical layer has been recovered.
    // [inner Exception, outer Exception] has one executable edge, but its
    // suffix identifies the enclosing try covering the inner catch body.
    // Reachability and catch selection use effective dispatch separately.
    let mut macros = BTreeMap::new();
    // One block may contribute several nested dispatch layers. Every installed
    // region removes at least one handler membership, including when its body
    // consists of just an earlier macro. Block count is not a progress bound.
    let memberships: usize = graph.blocks.iter().map(|block| block.handlers.len()).sum();
    for _ in 0..=memberships {
        let live = reachability_with_handlers(&graph);
        let reachable: Vec<_> = live
            .iter_enumerated()
            .filter_map(|(id, seen)| seen.then_some(id))
            .collect();
        let mut candidates: Vec<_> = reachable
            .iter()
            .copied()
            .filter(|&node| graph.blocks[node].may_throw && !graph.blocks[node].handlers.is_empty())
            .map(|node| (node, node))
            .collect();
        if candidates.is_empty() {
            let rejection = match super::recover_normal_flow(&graph, attempts) {
                Ok(body) => return Ok(expand(body, &macros)),
                Err(rejection) => rejection,
            };
            if allow_dispatch {
                attempts.push(RecoveryAttempt {
                    entry: graph.entry,
                    strategy: RecoveryStrategy::Dispatch,
                    rejection: None,
                });
                return Ok(residual_dispatch(&graph, &macros, &eligibility));
            }
            return Err(rejection);
        }
        let ownership = region_ownership(&graph, &live);
        for &node in &reachable {
            if let Some(anchor) = switch_coverage(&graph, node) {
                candidates.push((node, anchor));
            }
        }
        // Pure branches still delimit a useful try when their successors
        // perform protected operations. Preserve that lexical entry without
        // inventing handlers for isolated nonthrowing stores or returns.
        let mut included: IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
        for &(node, _) in &candidates {
            included[node] = true;
        }
        let mut cursor = 0;
        while cursor < candidates.len() {
            let (node, anchor) = candidates[cursor];
            cursor += 1;
            for &pred in &ownership.predecessors[node] {
                if included[pred] {
                    continue;
                }
                let handlers = &graph.blocks[anchor].handlers;
                if signature(&graph.blocks[pred].handlers, handlers)
                    || (!graph.blocks[pred].may_throw
                        && graph.blocks[pred].statements_empty
                        && control_coverage(&graph, pred, handlers, &eligibility))
                {
                    included[pred] = true;
                    candidates.push((pred, anchor));
                }
            }
        }
        // A split header may have a synthetic ID after all its descendants.
        // Recover dominating entries before their fragments at the same handler
        // depth; numeric block order is not a source-region ownership order.
        candidates.sort_by_key(|&(node, anchor)| {
            (
                std::cmp::Reverse(graph.blocks[anchor].handlers.len()),
                ownership.dom.preorder[node],
            )
        });
        let unresolved = candidates
            .iter()
            .copied()
            .map(|(node, _)| node)
            .find(|&node| graph.blocks[node].may_throw)
            .unwrap();
        let mut progress = false;
        for (entry, anchor) in candidates {
            let layer = Layer {
                entry,
                handlers: &graph.blocks[anchor].handlers,
                depth,
            };
            let Some(region) =
                candidate_with_eligibility(&graph, &live, &ownership, layer, &eligibility)
            else {
                continue;
            };
            debug_assert!(region.nodes.contains(&entry));
            debug_assert!(
                region
                    .nodes
                    .iter()
                    .map(|&node| graph.blocks[node].handlers.len())
                    .sum::<usize>()
                    > region.outer.len()
            );
            // Removing an impossible outer catch leaves a plain body, which
            // still owns previously recovered inner regions. Expand those
            // before this entry replaces its earlier macro, just as for Try.
            super::installation::install(
                &mut graph,
                &mut macros,
                super::installation::Region {
                    entry,
                    nodes: region.nodes,
                    outer: region.outer,
                    end: region.end,
                    body: super::installation::Body::Unexpanded(region.body),
                    // The region includes effects beyond an originally pure
                    // entry branch; retain its conservative throwing policy.
                    may_throw: true,
                },
                Some(&mut eligibility),
            );
            progress = true;
            break;
        }
        if !progress {
            let rejection = RecoveryRejection::at(unresolved, RejectionReason::ProtectedBlock);
            if allow_dispatch {
                attempts.push(RecoveryAttempt {
                    entry: graph.entry,
                    strategy: RecoveryStrategy::ExceptionRegions,
                    rejection: Some(rejection),
                });
                attempts.push(RecoveryAttempt {
                    entry: graph.entry,
                    strategy: RecoveryStrategy::Dispatch,
                    rejection: None,
                });
                return Ok(residual_dispatch(&graph, &macros, &eligibility));
            }
            return Err(rejection);
        }
    }
    Err(RecoveryRejection::at(
        graph.entry,
        RejectionReason::RegionBoundary,
    ))
}

fn arm_effect(graph: &FlowGraph, target: BlockId) -> BlockId {
    let block = &graph.blocks[target];
    match block.end {
        FlowEnd::Goto(next) if !block.may_throw => next,
        _ => target,
    }
}

/// Two protected arms anchor a lexical try around a pure switch. Dispatch on
/// its other arms must agree, or remain outside the proposed region.
fn switch_coverage(graph: &FlowGraph, node: BlockId) -> Option<BlockId> {
    let head = &graph.blocks[node];
    if head.may_throw || !head.statements_empty || !matches!(head.end, FlowEnd::Switch { .. }) {
        return None;
    }
    let arms = head.end.successors().into_iter().filter_map(|target| {
        let id = arm_effect(graph, target);
        let block = &graph.blocks[id];
        block.may_throw.then_some((id, block.handlers.as_slice()))
    });
    crate::cfg::dispatch::shared_arm(&head.handlers, arms)
}

/// Follow pure control and dispatch-neutral gaps to the first protected effect
/// on each path. A scheduled initialization must stay outside the lexical try.
fn control_coverage(
    graph: &FlowGraph,
    entry: BlockId,
    handlers: &[HandlerEdge],
    eligibility: &super::eligibility::Eligibility<'_>,
) -> bool {
    let mut seen: IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
    let mut pending = vec![entry];
    let mut effect = false;
    while let Some(node) = pending.pop() {
        if std::mem::replace(&mut seen[node], true) {
            continue;
        }
        let block = &graph.blocks[node];
        if block.may_throw {
            if signature(&block.handlers, handlers) {
                effect = true;
                continue;
            }
            if !eligibility.admits(node, &block.handlers, handlers) {
                return false;
            }
        } else if !block.statements_empty {
            return false;
        }
        pending.extend(block.end.successors());
    }
    effect
}

fn residual_dispatch(
    graph: &FlowGraph,
    macros: &BTreeMap<BlockId, Vec<Stmt>>,
    eligibility: &super::eligibility::Eligibility<'_>,
) -> Vec<Stmt> {
    expand(super::installation::dispatch(graph, eligibility), macros)
}

struct Region {
    nodes: Vec<BlockId>,
    outer: Vec<HandlerEdge>,
    end: FlowEnd,
    body: Vec<Stmt>,
    shared_catches: bool,
}

/// Handler entries are independent roots for normal-flow ownership. A join
/// reached by a catch therefore stays outside the protected body, while pure
/// branches and loop tests dominated by its entry can remain inside it.
struct Ownership {
    dom: Dominators,
    predecessors: IdVec<BlockId, Vec<BlockId>>,
}

fn region_ownership(graph: &FlowGraph, live: &IdVec<BlockId, bool>) -> Ownership {
    let root = graph.blocks.next_id();
    let mut successors: IdVec<BlockId, _> = graph
        .blocks
        .iter_enumerated()
        .map(|(node, block)| {
            if live[node] {
                block.end.successors()
            } else {
                Vec::new()
            }
        })
        .collect();
    let mut entries = vec![graph.entry];
    for (node, block) in graph.blocks.iter_enumerated() {
        if live[node] && block.may_throw {
            entries.extend(effective(&block.handlers).map(|handler| handler.target));
        }
    }
    entries.sort_unstable();
    entries.dedup();
    successors.push(entries);
    let mut predecessors: IdVec<BlockId, _> = vec![Vec::new(); successors.len()].into();
    for (source, targets) in successors.iter_enumerated() {
        for &target in targets {
            predecessors[target].push(source);
        }
    }
    let dom = Dominators::new(&successors, &predecessors, root);
    // The synthetic root was added last. Keep only normal predecessors for
    // subsequent region traversal.
    for &entry in &successors[root] {
        predecessors[entry].pop();
    }
    Ownership { dom, predecessors }
}

#[cfg(test)]
fn candidate(
    graph: &FlowGraph,
    live: &IdVec<BlockId, bool>,
    ownership: &Ownership,
    entry: BlockId,
    depth: usize,
) -> Option<Region> {
    candidate_with_eligibility(
        graph,
        live,
        ownership,
        Layer {
            entry,
            depth,
            handlers: &graph.blocks[entry].handlers,
        },
        &super::eligibility::Eligibility::default(),
    )
}

#[derive(Clone, Copy)]
struct Layer<'a> {
    entry: BlockId,
    handlers: &'a [HandlerEdge],
    depth: usize,
}

type Admit<'a> = dyn FnMut(BlockId, &[HandlerEdge], &[HandlerEdge]) -> bool + 'a;

fn candidate_with_eligibility(
    graph: &FlowGraph,
    live: &IdVec<BlockId, bool>,
    ownership: &Ownership,
    layer: Layer<'_>,
    eligibility: &super::eligibility::Eligibility<'_>,
) -> Option<Region> {
    let Layer {
        entry, handlers, ..
    } = layer;
    let first_handler = handlers.first()?;
    if first_handler.target == entry {
        return None;
    }
    let recover = |admits: &mut Admit<'_>| {
        let common = catch_outer_coverage(graph, ownership, first_handler.target, handlers, admits);
        let split = handlers.len() - common;
        if split == 0 {
            return None;
        }
        let nested = candidate_split(graph, live, ownership, layer, split, eligibility, admits);
        if split == handlers.len() || nested.as_ref().is_some_and(|region| !region.shared_catches) {
            return nested;
        }
        // A shared catch remains at its continuation. A single region may
        // consume the full table when separate fragments cannot own it.
        candidate_split(
            graph,
            live,
            ownership,
            layer,
            handlers.len(),
            eligibility,
            admits,
        )
        .or(nested)
    };
    let mut adopted = false;
    let trial = recover(&mut |node, short, long| {
        let safe = eligibility.admits(node, short, long);
        adopted |= safe;
        safe
    });
    if trial.is_none() && adopted {
        recover(&mut |_, _, _| false)
    } else {
        trial
    }
}

/// Coverage of effects in the catch's owned normal-flow region determines its
/// enclosing scope. A pure entry prefix contributes no coverage requirement.
/// An entirely nonthrowing catch can borrow lexical coverage from the normal
/// continuation it rejoins: no catch effect is moved across that boundary.
fn catch_outer_coverage(
    graph: &FlowGraph,
    ownership: &Ownership,
    entry: BlockId,
    handlers: &[HandlerEdge],
    admits: &mut Admit<'_>,
) -> usize {
    let mut seen: IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
    let mut pending = vec![entry];
    let mut frontiers = Vec::new();
    let shared_suffix = |edges: &[HandlerEdge]| {
        handlers
            .iter()
            .rev()
            .zip(edges.iter().rev())
            .take_while(|(a, b)| same(a, b))
            .count()
    };
    let mut common = handlers.len();
    let mut witnessed: Option<usize> = None;
    while let Some(node) = pending.pop() {
        if std::mem::replace(&mut seen[node], true) {
            continue;
        }
        if !ownership.dom.dominates(entry, node) {
            frontiers.push(node);
            continue;
        }
        let block = &graph.blocks[node];
        if block.may_throw {
            let exact = shared_suffix(&block.handlers);
            witnessed = witnessed.max(Some(exact));
            common = common.min(
                (exact + 1..=common)
                    .rev()
                    .find(|&n| admits(node, &block.handlers, &handlers[handlers.len() - n..]))
                    .unwrap_or(exact),
            );
            if common == 0 {
                break;
            }
        }
        pending.extend(block.end.successors());
    }
    if witnessed.is_some_and(|width| width >= common) {
        return common;
    }
    let mut frontier_suffix = None;
    // Empty catches often contain only an exception binding and a goto. Their
    // source outer range has no throwing instruction from which to retain a
    // handler edge, but the shared continuation still identifies that range.
    seen.fill(false);
    while let Some(node) = frontiers.pop() {
        if std::mem::replace(&mut seen[node], true) {
            continue;
        }
        let block = &graph.blocks[node];
        if block.may_throw {
            frontier_suffix = Some(
                frontier_suffix
                    .unwrap_or(handlers.len())
                    .min(shared_suffix(&block.handlers)),
            );
        } else {
            frontiers.extend(block.end.successors());
        }
    }
    // The first handler belongs to this catch even if the continuation retries
    // the same protected range. At least that one layer must be consumed.
    let evidence = witnessed
        .max(frontier_suffix)
        .unwrap_or_else(|| shared_suffix(&graph.blocks[entry].handlers));
    common.min(evidence).min(handlers.len().saturating_sub(1))
}

fn candidate_split(
    graph: &FlowGraph,
    live: &IdVec<BlockId, bool>,
    ownership: &Ownership,
    layer: Layer<'_>,
    split: usize,
    eligibility: &super::eligibility::Eligibility<'_>,
    admits: &mut Admit<'_>,
) -> Option<Region> {
    let Layer {
        entry,
        handlers,
        depth,
    } = layer;
    let outer = handlers[split..].to_vec();
    let mut selected: IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
    let mut pending = vec![entry];
    let anchored = |block: &FlowBlock| {
        block.may_throw
            && block
                .handlers
                .iter()
                .any(|actual| handlers[..split].iter().any(|caught| same(actual, caught)))
    };
    let mut gaps = Vec::new();
    while let Some(node) = pending.pop() {
        if selected[node] || !ownership.dom.dominates(entry, node) {
            continue;
        }
        let block = &graph.blocks[node];
        if block.may_throw && !signature(&block.handlers, handlers) {
            if !admits(node, &block.handlers, handlers) {
                continue;
            }
            if !anchored(block) {
                gaps.push(node);
            }
        }
        selected[node] = true;
        pending.extend(block.end.successors());
    }
    let mut boundary = Vec::new();
    if !gaps.is_empty() {
        // A remaining inner handler already puts this effect inside the try.
        // A completely uncovered effect must lead back to such an anchor;
        // trailing stores with no table of their own stay outside.
        let mut anchors: Vec<_> = graph
            .blocks
            .iter_enumerated()
            .filter_map(|(node, b)| (selected[node] && anchored(b)).then_some(node))
            .collect();
        let mut reaches: IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
        while let Some(node) = anchors.pop() {
            if selected[node] && !std::mem::replace(&mut reaches[node], true) {
                anchors.extend(&ownership.predecessors[node]);
            }
        }
        boundary.extend(gaps.into_iter().filter(|&node| !reaches[node]));
    }
    // A lexical try has one normal entry. External exceptional entries are
    // allowed only at its own header. A pure loop header may have been reached
    // before an incompatible throwing loop body; prune that shared frontier,
    // rather than rejecting the valid protected prefix that precedes it.
    for (source, block) in graph.blocks.iter_enumerated() {
        if selected[source] || !live[source] {
            continue;
        }
        boundary.extend(
            block
                .end
                .successors()
                .into_iter()
                .filter(|&target| selected[target] && target != entry),
        );
        if block.may_throw {
            boundary.extend(
                block
                    .handlers
                    .iter()
                    .map(|handler| handler.target)
                    .filter(|&target| selected[target] && target != entry),
            );
        }
    }
    while let Some(node) = boundary.pop() {
        if node == entry || !std::mem::replace(&mut selected[node], false) {
            continue;
        }
        boundary.extend(graph.blocks[node].end.successors());
    }
    let nodes: Vec<_> = selected
        .iter_enumerated()
        .filter_map(|(node, &yes)| yes.then_some(node))
        .collect();
    if !nodes.iter().any(|&node| {
        graph.blocks[node].may_throw && signature(&graph.blocks[node].handlers, handlers)
    }) {
        return None;
    }
    let caught = eligibility.caught(&nodes, &handlers[..split]);
    let mut exits = BTreeSet::new();
    for &node in &nodes {
        exits.extend(
            graph.blocks[node]
                .end
                .successors()
                .into_iter()
                .filter(|&target| !selected[target]),
        );
    }
    if caught.iter().any(|handler| selected[handler.target]) {
        return None;
    }
    // Catch paths may continue an enclosing loop or leave through a different
    // arm than normal execution. Their first meeting with ordinary method flow
    // is an exit from this lexical try, not part of the catch body.
    let mut normal: crate::index::IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
    let mut pending = vec![graph.entry];
    while let Some(node) = pending.pop() {
        if std::mem::replace(&mut normal[node], true) {
            continue;
        }
        pending.extend(graph.blocks[node].end.successors());
    }
    let mut shared = BTreeSet::new();
    for handler in &caught {
        let mut seen: IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
        let mut owned: crate::index::IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
        let mut handler_exits = BTreeSet::new();
        let mut inline = true;
        let mut pending = vec![handler.target];
        while let Some(node) = pending.pop() {
            if std::mem::replace(&mut seen[node], true) {
                continue;
            }
            if normal[node] || selected[node] {
                handler_exits.insert(node);
                continue;
            }
            owned[node] = true;
            if graph.blocks[node].may_throw {
                let Some(inner) = inner_handlers(node, &graph.blocks[node], &outer, admits) else {
                    inline = false;
                    break;
                };
                pending.extend(effective(inner).map(|handler| handler.target));
            }
            pending.extend(graph.blocks[node].end.successors());
        }
        // An inline catch owns its whole continuation, not merely its entry.
        // Otherwise recursively expanding nested/shared cleanup duplicates that
        // graph for every protected fragment. Keep the original handler as a
        // normal region continuation, with its exception context intact.
        if inline {
            for (source, block) in graph.blocks.iter_enumerated() {
                if !live[source] || selected[source] || owned[source] {
                    continue;
                }
                if block
                    .end
                    .successors()
                    .into_iter()
                    .any(|target| owned[target])
                    || (block.may_throw
                        && effective(&block.handlers).any(|edge| owned[edge.target]))
                {
                    inline = false;
                    break;
                }
            }
        }
        if inline && !handler_exits.contains(&handler.target) {
            exits.extend(handler_exits);
        } else {
            // Earlier recovered fragments may already expose this handler as
            // a normal continuation. An empty inline body at that boundary is
            // still shared ownership, not a newly exclusive catch.
            shared.insert(handler.target);
            exits.insert(handler.target);
        }
    }
    let multiple = exits.len() > 1;
    let exit_bodies: Vec<_> = exits
        .iter()
        .map(|&target| {
            (
                target,
                if multiple {
                    vec![Stmt::Jump(target)]
                } else {
                    Vec::new()
                },
            )
        })
        .collect();
    // A catch may retry the protected header. That is an exit of the catch,
    // while normal backedges inside the body still belong to the body itself.
    let body_exits: Vec<_> = exit_bodies
        .iter()
        .filter(|(target, _)| !selected[*target])
        .cloned()
        .collect();
    let body = subgraph_with_exits(
        graph.clone(),
        entry,
        &body_exits,
        &selected,
        Some((handlers, &mut *admits)),
        depth + 1,
        |_| eligibility.clone(),
    )?;
    let mut catches = Vec::new();
    let catch_allowed: IdVec<BlockId, _> = selected.iter().map(|selected| !selected).collect();
    for handler in &caught {
        let catch_body = if shared.contains(&handler.target) {
            exit_bodies
                .iter()
                .find(|(target, _)| *target == handler.target)?
                .1
                .clone()
        } else {
            subgraph_with_exits(
                graph.clone(),
                handler.target,
                &exit_bodies,
                &catch_allowed,
                Some((&outer, &mut *admits)),
                depth + 1,
                |_| eligibility.clone(),
            )?
        };
        // Reentry is represented only by an explicit boundary transfer. A
        // catch must not duplicate the protected body's executable statements.
        if references_any(&catch_body, &selected) {
            return None;
        }
        catches.push(Catch {
            catch_type: handler.catch_type.clone(),
            handler: handler.target,
            body: catch_body,
        });
    }
    Some(Region {
        nodes,
        outer,
        end: if multiple {
            FlowEnd::Multi(exits.into_iter().map(|target| (target, target)).collect())
        } else {
            exits
                .first()
                .copied()
                .map_or(FlowEnd::Unreachable, FlowEnd::Goto)
        },
        body: if catches.is_empty() {
            body
        } else {
            vec![Stmt::Try { body, catches }]
        },
        shared_catches: !shared.is_empty(),
    })
}

fn inner_handlers<'a>(
    node: BlockId,
    block: &'a FlowBlock,
    outer: &[HandlerEdge],
    admits: &mut Admit<'_>,
) -> Option<&'a [HandlerEdge]> {
    crate::cfg::dispatch::strip_outer(&block.handlers, outer)
        .or_else(|| (!block.may_throw || admits(node, &block.handlers, outer)).then_some(&[]))
}

pub(super) fn subgraph_with_exits<'a>(
    mut sub: FlowGraph,
    entry: BlockId,
    exits: &[(BlockId, Vec<Stmt>)],
    allowed: &IdVec<BlockId, bool>,
    mut remove: Option<(&[HandlerEdge], &mut Admit<'_>)>,
    depth: usize,
    source_handlers: impl FnOnce(&FlowGraph) -> super::eligibility::Eligibility<'a>,
) -> Option<Vec<Stmt>> {
    if let Some((_, body)) = exits.iter().find(|(target, _)| *target == entry) {
        return Some(body.clone());
    }
    sub.entry = entry;
    let boundary = sub.blocks.next_id();
    // A subgraph exit stands for a control transfer. Treating it as a
    // throwing block would split outer catches when an inner handler exits.
    let mut replacements = BTreeMap::new();
    for (index, (_, body)) in exits.iter().enumerate() {
        sub.blocks.push(FlowBlock {
            pc: u32::MAX,
            statements_empty: false,
            duplication_cost: None,
            is_loop_update: false,
            end: FlowEnd::Return,
            handlers: Vec::new(),
            may_throw: false,
        });
        replacements.insert(BlockId::from_index(boundary.index() + index), body.clone());
    }
    let mut seen: IdVec<BlockId, _> = vec![false; sub.blocks.len()].into();
    let mut pending = vec![entry];
    while let Some(node) = pending.pop() {
        if seen[node] {
            continue;
        }
        if !allowed[node] {
            return None;
        }
        seen[node] = true;
        let block = &mut sub.blocks[node];
        if let Some((outer, admits)) = remove.as_mut() {
            let keep = inner_handlers(node, block, outer, *admits)?.len();
            block.handlers.truncate(keep);
        }
        block.end.map_destinations(|target| {
            exits
                .iter()
                .position(|(exit, _)| *exit == target)
                .map_or(target, |index| {
                    BlockId::from_index(boundary.index() + index)
                })
        });
        for target in block.end.successors() {
            if target >= boundary {
                seen[target] = true;
            } else {
                pending.push(target);
            }
        }
        if block.may_throw {
            pending.extend(effective(&block.handlers).map(|handler| handler.target));
        }
    }
    for (node, block) in sub.blocks.iter_enumerated_mut() {
        if !seen[node] {
            super::installation::tombstone(block);
        }
    }
    let mut eligibility = source_handlers(&sub);
    for &exit in replacements.keys() {
        eligibility.boundary(exit);
    }
    // This is a speculative candidate. Only the committed enclosing strategy
    // reports its rejection; rolled-back probes do not enter method history.
    let mut body = recover_inner(sub, depth, false, eligibility, &mut Vec::new()).ok()?;
    let lexical_boundary = replacements.values().any(Vec::is_empty);
    if lexical_boundary {
        super::renumber_regions_from(&mut body, 1);
        for replacement in replacements.values_mut() {
            if replacement.is_empty() {
                replacement.push(Stmt::Break { target: 0 });
            }
        }
    }
    let body = expand(body, &replacements);
    Some(if lexical_boundary && super::contains_break(&body, 0) {
        super::stackify::simplify(vec![Stmt::Label { region: 0, body }])
    } else {
        body
    })
}

fn references_any(body: &[Stmt], selected: &IdVec<BlockId, bool>) -> bool {
    body.iter().any(|stmt| match stmt {
        Stmt::TryFinally { body, cleanup, .. } => {
            references_any(body, selected) || references_any(cleanup, selected)
        }
        Stmt::Basic(node)
        | Stmt::Statements { block: node, .. }
        | Stmt::Return(node)
        | Stmt::Throw(node)
        | Stmt::Unreachable(node) => selected[*node],
        Stmt::If {
            then_body,
            else_body,
            ..
        } => references_any(then_body, selected) || references_any(else_body, selected),
        Stmt::Loop {
            body, continuation, ..
        } => continuation.is_some_and(|node| selected[node]) || references_any(body, selected),
        Stmt::Label { body, .. } | Stmt::Synchronized { body, .. } => {
            references_any(body, selected)
        }
        Stmt::Switch { cases, .. } => cases
            .iter()
            .any(|case| references_any(&case.body, selected)),
        Stmt::Try { body, catches } => {
            references_any(body, selected)
                || catches
                    .iter()
                    .any(|catch| references_any(&catch.body, selected))
        }
        _ => false,
    })
}
