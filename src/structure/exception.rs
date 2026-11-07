//! Recover lexical try regions from ordered dispatch, keeping catches in table
//! order and leaving duplicated cleanup as ordinary code. In particular a
//! catch-all is not evidence that every normal exit executes a finally body.

use super::analysis::{Dominators, reachable_with_handlers};
use super::attempt::{
    self, RecoveryAttempt, RecoveryRejection, RecoveryResult, RecoveryStrategy, RejectionReason,
};
use super::installation::expand;
use super::{Catch, FlowBlock, FlowEnd, FlowGraph, Stmt};
use crate::cfg::HandlerEdge;
use crate::cfg::dispatch::{effective, same_dispatch as signature, same_handler as same};
use crate::index::{BlockId, IdVec};
use std::collections::{BTreeMap, BTreeSet};

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
    match recover_inner(graph, 0, true, eligibility, attempts) {
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
    graph: &FlowGraph,
    depth: usize,
    allow_dispatch: bool,
    eligibility: &super::eligibility::Eligibility<'_>,
    attempts: &mut Vec<RecoveryAttempt>,
) -> RecoveryResult<Vec<Stmt>> {
    if depth > 48 {
        return Err(RecoveryRejection::at(
            graph.entry,
            RejectionReason::NestingLimit,
        ));
    }
    let mut graph = graph.clone();
    let mut eligibility = eligibility.clone();
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
        let reachable = reachable_with_handlers(&graph);
        let mut live: IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
        for &node in &reachable {
            live[node] = true;
        }
        let ownership = region_ownership(&graph, &live);
        let mut candidates: Vec<_> = reachable
            .iter()
            .copied()
            .filter(|&node| graph.blocks[node].may_throw && !graph.blocks[node].handlers.is_empty())
            .collect();
        // Pure branches still delimit a useful try when their successors
        // perform protected operations. Preserve that lexical entry without
        // inventing handlers for isolated nonthrowing stores or returns.
        let mut predecessors: IdVec<BlockId, _> = vec![Vec::new(); graph.blocks.len()].into();
        for &source in &reachable {
            for target in graph.blocks[source].end.successors() {
                predecessors[target].push(source);
            }
        }
        let mut included: IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
        for &node in &candidates {
            included[node] = true;
        }
        let mut cursor = 0;
        while cursor < candidates.len() {
            let node = candidates[cursor];
            cursor += 1;
            for &pred in &predecessors[node] {
                if included[pred] {
                    continue;
                }
                let matching =
                    signature(&graph.blocks[pred].handlers, &graph.blocks[node].handlers);
                if matching
                    || (!graph.blocks[pred].may_throw
                        && graph.blocks[pred].statements_empty
                        && control_coverage(&graph, pred, &graph.blocks[node].handlers))
                {
                    if !matching {
                        // Pure control can be the common entry of several
                        // protected arms even after preparation discarded its
                        // own table edges. A scheduled initialization remains
                        // outside the try for Java's definite-assignment rule.
                        graph.blocks[pred].handlers = graph.blocks[node].handlers.clone();
                    }
                    included[pred] = true;
                    candidates.push(pred);
                }
            }
        }
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
        // A split header may have a synthetic ID after all its descendants.
        // Recover dominating entries before their fragments at the same handler
        // depth; numeric block order is not a source-region ownership order.
        candidates.sort_by_key(|&node| {
            (
                std::cmp::Reverse(graph.blocks[node].handlers.len()),
                ownership.preorder[node],
            )
        });
        let unresolved = candidates
            .iter()
            .copied()
            .find(|&node| graph.blocks[node].may_throw)
            .unwrap();
        let mut progress = false;
        for entry in candidates {
            let Some(region) =
                candidate_with_eligibility(&graph, &live, &ownership, entry, depth, &eligibility)
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

/// The first effect on every normal path has the same protection. Reading
/// beyond that effect would incorrectly merge a later, different try scope.
fn control_coverage(graph: &FlowGraph, entry: BlockId, handlers: &[HandlerEdge]) -> bool {
    let mut seen: IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
    let mut pending = vec![entry];
    let mut effect = false;
    while let Some(node) = pending.pop() {
        if std::mem::replace(&mut seen[node], true) {
            continue;
        }
        let block = &graph.blocks[node];
        if block.may_throw {
            if !signature(&block.handlers, handlers) {
                return false;
            }
            effect = true;
        } else if block.statements_empty {
            pending.extend(block.end.successors());
        } else {
            return false;
        }
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
fn region_ownership(graph: &FlowGraph, live: &IdVec<BlockId, bool>) -> Dominators {
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
    Dominators::new(&successors, &predecessors, root)
}

#[cfg(test)]
fn candidate(
    graph: &FlowGraph,
    live: &IdVec<BlockId, bool>,
    ownership: &Dominators,
    entry: BlockId,
    depth: usize,
) -> Option<Region> {
    candidate_with_eligibility(
        graph,
        live,
        ownership,
        entry,
        depth,
        &super::eligibility::Eligibility::default(),
    )
}

fn candidate_with_eligibility(
    graph: &FlowGraph,
    live: &IdVec<BlockId, bool>,
    ownership: &Dominators,
    entry: BlockId,
    depth: usize,
    eligibility: &super::eligibility::Eligibility<'_>,
) -> Option<Region> {
    let handlers = &graph.blocks[entry].handlers;
    let first_handler = handlers.first()?;
    if first_handler.target == entry {
        return None;
    }
    let common = catch_outer_coverage(graph, ownership, first_handler.target, handlers);
    let split = handlers.len() - common;
    if split == 0 {
        return None;
    }
    let nested = candidate_split(graph, live, ownership, entry, depth, split, eligibility);
    if split == handlers.len() || nested.as_ref().is_some_and(|region| !region.shared_catches) {
        return nested;
    }
    // A shared catch is an explicit region continuation, so its enclosing
    // coverage does not force a wrapper around each protected fragment. Try
    // consuming the full dispatch once; the existing ownership proof retains
    // that catch and its outer handlers at their original continuation.
    candidate_split(
        graph,
        live,
        ownership,
        entry,
        depth,
        handlers.len(),
        eligibility,
    )
    .or(nested)
}

/// Coverage of effects in the catch's owned normal-flow region determines its
/// enclosing scope. A pure entry prefix contributes no coverage requirement.
/// An entirely nonthrowing catch can borrow lexical coverage from the normal
/// continuation it rejoins: no catch effect is moved across that boundary.
fn catch_outer_coverage(
    graph: &FlowGraph,
    ownership: &Dominators,
    entry: BlockId,
    handlers: &[HandlerEdge],
) -> usize {
    let mut seen: IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
    let mut pending = vec![entry];
    let mut frontiers = Vec::new();
    let mut common = handlers.len();
    let mut has_effect = false;
    while let Some(node) = pending.pop() {
        if std::mem::replace(&mut seen[node], true) {
            continue;
        }
        if !ownership.dominates(entry, node) {
            frontiers.push(node);
            continue;
        }
        let block = &graph.blocks[node];
        if block.may_throw {
            has_effect = true;
            common = common.min(
                handlers
                    .iter()
                    .rev()
                    .zip(block.handlers.iter().rev())
                    .take_while(|(a, b)| same(a, b))
                    .count(),
            );
            if common == 0 {
                break;
            }
        }
        pending.extend(block.end.successors());
    }
    if has_effect {
        return common;
    }
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
            has_effect = true;
            common = common.min(
                handlers
                    .iter()
                    .rev()
                    .zip(block.handlers.iter().rev())
                    .take_while(|(a, b)| same(a, b))
                    .count(),
            );
        } else {
            frontiers.extend(block.end.successors());
        }
    }
    // The first handler belongs to this catch even if the continuation retries
    // the same protected range. At least that one layer must be consumed.
    if has_effect {
        common.min(handlers.len().saturating_sub(1))
    } else {
        0
    }
}

fn candidate_split(
    graph: &FlowGraph,
    live: &IdVec<BlockId, bool>,
    ownership: &Dominators,
    entry: BlockId,
    depth: usize,
    split: usize,
    eligibility: &super::eligibility::Eligibility<'_>,
) -> Option<Region> {
    let handlers = &graph.blocks[entry].handlers;
    let outer = handlers[split..].to_vec();
    let mut selected: IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
    let mut pending = vec![entry];
    let mut exits = BTreeSet::new();
    while let Some(node) = pending.pop() {
        if selected[node] {
            continue;
        }
        // Exception coverage constrains effects, not the nonthrowing control
        // between them. Ownership keeps shared continuations outside the try.
        if !ownership.dominates(entry, node)
            || (graph.blocks[node].may_throw && !signature(&graph.blocks[node].handlers, handlers))
        {
            exits.insert(node);
            continue;
        }
        selected[node] = true;
        pending.extend(graph.blocks[node].end.successors());
    }
    // A lexical try has one normal entry. External exceptional entries are
    // allowed only at its own header. A pure loop header may have been reached
    // before an incompatible throwing loop body; prune that shared frontier,
    // rather than rejecting the valid protected prefix that precedes it.
    let mut boundary = Vec::new();
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
    if !nodes.iter().any(|&node| graph.blocks[node].may_throw) {
        return None;
    }
    let caught = eligibility.caught(&nodes, &handlers[..split]);
    exits.clear();
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
                let coverage = &graph.blocks[node].handlers;
                if coverage.len() < outer.len()
                    || !signature(&coverage[coverage.len() - outer.len()..], &outer)
                {
                    inline = false;
                    break;
                }
                pending.extend(
                    effective(&coverage[..coverage.len() - outer.len()])
                        .map(|handler| handler.target),
                );
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
        graph,
        entry,
        &body_exits,
        Some(&selected),
        handlers,
        depth + 1,
        false,
        eligibility,
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
                graph,
                handler.target,
                &exit_bodies,
                Some(&catch_allowed),
                &outer,
                depth + 1,
                false,
                eligibility,
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

#[expect(
    clippy::too_many_arguments,
    reason = "Subgraph boundaries, handler removal, and recovery policy are independent caller inputs"
)]
pub(super) fn subgraph_with_exits(
    graph: &FlowGraph,
    entry: BlockId,
    exits: &[(BlockId, Vec<Stmt>)],
    allowed: Option<&IdVec<BlockId, bool>>,
    remove: &[HandlerEdge],
    depth: usize,
    allow_dispatch: bool,
    eligibility: &super::eligibility::Eligibility<'_>,
) -> Option<Vec<Stmt>> {
    if let Some((_, body)) = exits.iter().find(|(target, _)| *target == entry) {
        return Some(body.clone());
    }
    let boundary = graph.blocks.next_id();
    let mut eligibility = eligibility.clone();
    let mut sub = FlowGraph {
        entry,
        blocks: graph.blocks.clone(),
    };
    // The analysis-only boundary is never absorbed as a nonthrowing connector.
    let mut replacements = BTreeMap::new();
    for (index, (_, body)) in exits.iter().enumerate() {
        sub.blocks.push(FlowBlock {
            pc: u32::MAX,
            statements_empty: false,
            duplication_cost: None,
            is_loop_update: false,
            end: FlowEnd::Return,
            handlers: Vec::new(),
            may_throw: true,
        });
        replacements.insert(BlockId::from_index(boundary.index() + index), body.clone());
        eligibility.boundary(BlockId::from_index(boundary.index() + index));
    }
    let mut seen: IdVec<BlockId, _> = vec![false; sub.blocks.len()].into();
    let mut pending = vec![entry];
    while let Some(node) = pending.pop() {
        if seen[node] {
            continue;
        }
        if allowed.is_some_and(|allowed| !allowed[node]) {
            return None;
        }
        seen[node] = true;
        let block = &mut sub.blocks[node];
        if block.handlers.len() >= remove.len()
            && signature(
                &block.handlers[block.handlers.len() - remove.len()..],
                remove,
            )
        {
            block.handlers.truncate(block.handlers.len() - remove.len());
        } else if !block.may_throw {
            block.handlers.clear();
        } else {
            return None;
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
    // This is a speculative candidate. Only the committed enclosing strategy
    // reports its rejection; rolled-back probes do not enter method history.
    let mut body =
        recover_inner(&sub, depth, allow_dispatch, &eligibility, &mut Vec::new()).ok()?;
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
