//! A bounded source overlay for a shared linear prefix at two loop entries.
//! Hierarchical SCC certificates produce one overlay, tried once after ordinary
//! recovery and stackification both fail.
use crate::index::IdVec;
use std::collections::BTreeMap;

use super::attempt::{RecoveryRejection, RecoveryResult, RejectionReason};
use super::{BlockId, FlowEnd, FlowGraph, Stmt, analysis::Analysis};

// Bound actual expanded source work, including inline expression descendants.
// Each accepted original prefix is copied once; there is no iterative cloning.
const PREFIX_COST_LIMIT: usize = super::duplication::BLOCK_COST_LIMIT;
const METHOD_COST_LIMIT: usize = 192;

pub(super) fn recover(graph: &FlowGraph, analysis: &Analysis) -> RecoveryResult<Vec<Stmt>> {
    let mut predecessors: IdVec<BlockId, _> = vec![Vec::new(); graph.blocks.len()].into();
    let mut residual: IdVec<BlockId, _> = vec![Vec::new(); graph.blocks.len()].into();
    let mut handler_entries: IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
    for (source, targets) in analysis.successors.iter_enumerated() {
        if !analysis.reachable[source] {
            continue;
        }
        for &target in targets {
            // Natural-loop backedges are already representable. Their removal
            // isolates irreducible inner cycles even inside an ordinary loop.
            if !analysis.loop_at[target].is_some_and(|index| analysis.loops[index].contains(source))
            {
                predecessors[target].push(source);
                residual[source].push(target);
            }
        }
    }
    for source in super::analysis::reachable_with_handlers(graph) {
        for handler in &graph.blocks[source].handlers {
            handler_entries[handler.target] = true;
        }
    }
    let mut components: Vec<_> = components(&residual, &analysis.reachable)
        .into_iter()
        .filter(|nodes| nodes.len() > 1)
        .collect();
    let mut rejected_entry = graph.entry;
    let mut membership: IdVec<BlockId, _> = vec![usize::MAX; graph.blocks.len()].into();
    let mut split_original: IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
    let mut splits = Vec::new();
    let mut total_cost = 0;
    let mut id = 0;
    while let Some(nodes) = components.pop() {
        id += 1;
        for &node in &nodes {
            membership[node] = id;
        }
        let entries: Vec<_> = nodes
            .iter()
            .copied()
            .filter(|&node| {
                node == graph.entry
                    || predecessors[node]
                        .iter()
                        .any(|&source| membership[source] != id)
            })
            .collect();
        if let Some(&entry) = entries.first() {
            rejected_entry = entry;
        }
        if entries.len() != 2 {
            continue;
        }
        for (prefix, header) in [(entries[0], entries[1]), (entries[1], entries[0])] {
            let data = &graph.blocks[prefix];
            let Some(cost) = data.duplication_cost else {
                continue;
            };
            if cost as usize > PREFIX_COST_LIMIT
                || total_cost + cost as usize > METHOD_COST_LIMIT
                || split_original[prefix]
                || prefix == graph.entry
                || handler_entries[prefix]
                || handler_entries[header]
                || !data.handlers.is_empty()
                || !matches!(data.end, FlowEnd::Goto(target) if target == header)
            {
                continue;
            }
            let incoming: Vec<_> = predecessors[prefix]
                .iter()
                .copied()
                .filter(|&source| membership[source] != id)
                .collect();
            if incoming.is_empty() {
                continue;
            }
            splits.push((prefix, incoming));
            split_original[prefix] = true;
            total_cost += cost as usize;
            // Redirecting the external prefix entries through its clone makes
            // `header` the sole entry of this SCC. Its internal incoming edges
            // are therefore backedges of a now-certified enclosing loop. Cut
            // only those edges in the planning graph to expose nested SCCs.
            // New clone paths enter this header; all of its outgoing edges
            // remain real predecessors of any nested candidate.
            // The header can be an opaque semantic region: its conditional
            // return/throw exits need not appear as a Branch in this graph.
            for &source in &nodes {
                residual[source].retain(|&target| target != header);
            }
            predecessors[header].retain(|&source| membership[source] != id);
            let mut inside: IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
            for &node in &nodes {
                inside[node] = node != header;
            }
            components.extend(
                self::components(&residual, &inside)
                    .into_iter()
                    .filter(|nodes| nodes.len() > 1),
            );
            break;
        }
    }
    if splits.is_empty() {
        return Err(RecoveryRejection::at(
            rejected_entry,
            RejectionReason::EntryPrefix,
        ));
    }
    let mut overlay = graph.clone();
    let mut originals = BTreeMap::new();
    for (prefix, incoming) in splits {
        let clone = overlay.blocks.next_id();
        let mut data = graph.blocks[prefix].clone();
        // The clone is still a proven linear prefix. Its incoming paths may
        // need separate source occurrences inside branch arms; ordinary tail
        // recovery owns and charges that work with its method-wide bound.
        data.statements_empty = false;
        overlay.blocks.push(data);
        overlay.blocks[prefix].duplication_cost = None;
        originals.insert(clone, prefix);
        for source in incoming {
            redirect(&mut overlay.blocks[source].end, prefix, clone);
        }
    }
    let analysis = Analysis::new(&overlay);
    let body = super::recover::recover(&overlay, &analysis)
        .or_else(|_| super::stackify::stackify(&overlay, &analysis))
        .map_err(|mut rejection| {
            // Diagnostic locations refer to source blocks, never overlay IDs.
            rejection.block = originals
                .get(&rejection.block)
                .copied()
                .unwrap_or(rejection.block);
            if let RejectionReason::LoopExit { header } = &mut rejection.reason {
                *header = originals.get(header).copied().unwrap_or(*header);
            }
            rejection
        })?;
    let originals = originals
        .into_iter()
        .map(|(clone, original)| (clone, vec![Stmt::Basic(original)]))
        .collect();
    Ok(super::installation::expand(body, &originals))
}

fn redirect(end: &mut FlowEnd, from: BlockId, to: BlockId) {
    end.map_destinations(|target| if target == from { to } else { target });
}

/// Iterative Kosaraju traversal avoids recursion on hostile control-flow graphs.
fn components(
    successors: &IdVec<BlockId, Vec<BlockId>>,
    reachable: &IdVec<BlockId, bool>,
) -> Vec<Vec<BlockId>> {
    let mut reverse: IdVec<BlockId, _> = vec![Vec::new(); successors.len()].into();
    for (source, targets) in successors.iter_enumerated() {
        for &target in targets {
            if reachable[source] && reachable[target] {
                reverse[target].push(source);
            }
        }
    }
    let mut seen: IdVec<BlockId, _> = vec![false; successors.len()].into();
    let mut order = Vec::new();
    for root in successors.ids() {
        if !reachable[root] || seen[root] {
            continue;
        }
        seen[root] = true;
        let mut pending = vec![(root, 0)];
        while let Some((node, next)) = pending.last_mut() {
            if let Some(&target) = successors[*node].get(*next) {
                *next += 1;
                if reachable[target] && !std::mem::replace(&mut seen[target], true) {
                    pending.push((target, 0));
                }
            } else {
                order.push(*node);
                pending.pop();
            }
        }
    }
    seen.fill(false);
    let mut result = Vec::new();
    for &root in order.iter().rev() {
        if seen[root] {
            continue;
        }
        let mut nodes = Vec::new();
        let mut pending = vec![root];
        seen[root] = true;
        while let Some(node) = pending.pop() {
            nodes.push(node);
            for &source in &reverse[node] {
                if !std::mem::replace(&mut seen[source], true) {
                    pending.push(source);
                }
            }
        }
        result.push(nodes);
    }
    result
}
