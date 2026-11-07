use super::{BlockId, FlowEnd, FlowGraph};
pub(super) use crate::cfg::Dominators;
use crate::index::IdVec;

pub(super) struct Analysis {
    pub successors: IdVec<BlockId, Vec<BlockId>>,
    pub predecessors: IdVec<BlockId, Vec<BlockId>>,
    pub handler_entries: IdVec<BlockId, bool>,
    pub reachable: IdVec<BlockId, bool>,
    pub postdom: Dominators,
    pub loops: Vec<LoopRegion>,
    pub loop_at: IdVec<BlockId, Option<usize>>,
}

#[derive(Clone, Debug)]
pub(super) struct LoopRegion {
    pub header: BlockId,
    pub nodes: Vec<BlockId>,
    pub exits: Vec<BlockId>,
    pub parent: Option<usize>,
}

impl LoopRegion {
    pub fn contains(&self, block: BlockId) -> bool {
        self.nodes.binary_search(&block).is_ok()
    }
}

pub(super) fn validate(graph: &FlowGraph) -> crate::Result<()> {
    anyhow::ensure!(
        graph.blocks.is_empty() || graph.blocks.contains_id(graph.entry),
        "control-flow entry is out of range"
    );
    for (id, block) in graph.blocks.iter_enumerated() {
        for target in block.end.successors() {
            anyhow::ensure!(
                graph.blocks.contains_id(target),
                "block {id} targets missing block {target}"
            );
        }
        for handler in &block.handlers {
            anyhow::ensure!(
                graph.blocks.contains_id(handler.target),
                "block {id} targets missing exception handler {}",
                handler.target
            );
        }
    }
    Ok(())
}

impl Analysis {
    pub fn new(graph: &FlowGraph) -> Self {
        let n = graph.blocks.len();
        let successors: IdVec<BlockId, _> = graph
            .blocks
            .iter()
            .map(|block| block.end.successors())
            .collect();
        let mut predecessors: IdVec<BlockId, _> = vec![Vec::new(); n].into();
        for (source, targets) in successors.iter_enumerated() {
            for &target in targets {
                predecessors[target].push(source);
            }
        }
        let dom = Dominators::new(&successors, &predecessors, graph.entry);
        let reachable: IdVec<BlockId, _> = dom.idom.iter().map(Option::is_some).collect();

        // All real exits share a synthetic successor. Non-terminating sink SCCs
        // also reach it for analysis only, so an infinite arm never accidentally
        // postdominates code on a returning arm.
        let virtual_exit = successors.next_id();
        let mut reverse = predecessors.clone();
        let mut forward = successors.clone();
        reverse.push(Vec::new());
        forward.push(Vec::new());
        let mut reaches_exit: IdVec<BlockId, _> = vec![false; n].into();
        let mut pending = Vec::new();
        for (id, targets) in successors.iter_enumerated() {
            if targets.is_empty() && reachable[id] {
                pending.push(id);
                reverse[virtual_exit].push(id);
                forward[id].push(virtual_exit);
            }
        }
        while let Some(id) = pending.pop() {
            if std::mem::replace(&mut reaches_exit[id], true) {
                continue;
            }
            pending.extend(predecessors[id].iter().copied());
        }
        for id in graph.blocks.ids() {
            if reachable[id] && !reaches_exit[id] {
                reverse[virtual_exit].push(id);
                forward[id].push(virtual_exit);
            }
        }
        let postdom = Dominators::new(&reverse, &forward, virtual_exit);

        let mut by_header: IdVec<BlockId, Vec<BlockId>> = vec![Vec::new(); n].into();
        let mut marks: IdVec<BlockId, _> = vec![0usize; n].into();
        let mut stamp = 0;
        for (tail, targets) in successors.iter_enumerated() {
            if !reachable[tail] {
                continue;
            }
            for &head in targets {
                if !dom.dominates(head, tail) {
                    continue;
                }
                stamp += 1;
                marks[head] = stamp;
                let mut nodes = vec![head];
                let mut pending = vec![tail];
                while let Some(node) = pending.pop() {
                    if marks[node] == stamp || !dom.dominates(head, node) {
                        continue;
                    }
                    marks[node] = stamp;
                    nodes.push(node);
                    pending.extend(predecessors[node].iter().copied());
                }
                by_header[head].extend(nodes);
            }
        }
        let mut loops = Vec::new();
        let mut loop_at: IdVec<BlockId, _> = vec![None; n].into();
        for (header, nodes) in by_header.iter_enumerated_mut() {
            if nodes.is_empty() {
                continue;
            }
            nodes.sort_unstable();
            nodes.dedup();
            let mut exits = Vec::new();
            for &node in nodes.iter() {
                for &target in &successors[node] {
                    if nodes.binary_search(&target).is_err() {
                        exits.push(target);
                    }
                }
            }
            exits.sort_unstable();
            exits.dedup();
            loop_at[header] = Some(loops.len());
            loops.push(LoopRegion {
                header,
                nodes: std::mem::take(nodes),
                exits,
                parent: None,
            });
        }
        for index in 0..loops.len() {
            loops[index].parent = (0..loops.len())
                .filter(|&other| {
                    other != index
                        && loops[other].nodes.len() > loops[index].nodes.len()
                        && loops[other].contains(loops[index].header)
                })
                .min_by_key(|&other| loops[other].nodes.len());
        }
        let mut handler_entries: IdVec<BlockId, _> = vec![false; n].into();
        for source in reachable_with_handlers(graph) {
            for handler in &graph.blocks[source].handlers {
                handler_entries[handler.target] = true;
            }
        }
        let mut result = Self {
            successors,
            predecessors,
            handler_entries,
            reachable,
            postdom,
            loops,
            loop_at,
        };
        // Natural backedges omit one-shot exit work. Own that work up to the
        // common continuation before either recovery strategy chooses a loop
        // body; topological order alone cannot identify a loop's follow.
        for index in 0..result.loops.len() {
            let lp = &result.loops[index];
            if lp.exits.len() < 2 {
                continue;
            }
            let Some(follow) = result.common_postdom(lp.exits.iter().copied()) else {
                continue;
            };
            if lp.contains(follow) {
                continue;
            }
            // The guard's normal continuation stays outside the loop even
            // when it contains work before the shared follow (for example a
            // second search). That does not prevent owning an independent
            // successful exit. Keep both destinations as boundaries: lexical
            // recovery can express success as an outward break and exhaustion
            // as ordinary loop completion, without detaching the success work.
            let guard = result.loop_guard(graph, index);
            let Some(nodes) =
                result.exit_region(graph, index, |node| node == follow || Some(node) == guard)
            else {
                continue;
            };
            if lp.parent.is_some_and(|parent| {
                nodes
                    .iter()
                    .any(|&node| !result.loops[parent].contains(node))
            }) {
                continue;
            }
            result.loops[index].nodes.extend(nodes);
            result.loops[index].nodes.sort_unstable();
            let lp = &result.loops[index];
            let mut exits: Vec<_> = lp
                .nodes
                .iter()
                .flat_map(|&node| result.successors[node].iter().copied())
                .filter(|&target| !lp.contains(target))
                .collect();
            exits.sort_unstable();
            exits.dedup();
            result.loops[index].exits = exits;
        }
        result
    }

    pub fn loop_guard(&self, graph: &FlowGraph, index: usize) -> Option<BlockId> {
        let lp = &self.loops[index];
        let exit = |block: BlockId| {
            let FlowEnd::Branch {
                then_target,
                else_target,
            } = graph.blocks[block].end
            else {
                return None;
            };
            match (lp.contains(then_target), lp.contains(else_target)) {
                (true, false) => Some(else_target),
                (false, true) => Some(then_target),
                _ => None,
            }
        };
        exit(lp.header).or_else(|| {
            lp.nodes
                .iter()
                .copied()
                .filter(|&node| self.successors[node].contains(&lp.header))
                .find_map(exit)
        })
    }

    pub fn common_postdom(&self, entries: impl IntoIterator<Item = BlockId>) -> Option<BlockId> {
        self.postdom
            .common(entries)
            .filter(|&common| self.successors.contains_id(common))
    }

    /// All exit paths share one ownership proof. Shared corridors belong once
    /// to the loop, while incoming edges from another region reject absorption.
    pub fn exit_region(
        &self,
        graph: &FlowGraph,
        index: usize,
        boundary: impl Fn(BlockId) -> bool,
    ) -> Option<Vec<BlockId>> {
        let lp = &self.loops[index];
        let mut color: IdVec<BlockId, _> = vec![0u8; graph.blocks.len()].into();
        let mut nodes = Vec::new();
        for &entry in &lp.exits {
            if boundary(entry) || color[entry] == 2 {
                continue;
            }
            let mut pending = vec![(entry, 0)];
            while let Some((node, next)) = pending.last_mut() {
                if color[*node] == 0 {
                    if lp.contains(*node)
                        || self.handler_entries[*node]
                        || graph.blocks[*node].may_throw
                            && graph.blocks[*node].handlers != graph.blocks[lp.header].handlers
                        || matches!(graph.blocks[*node].end, FlowEnd::Multi(_))
                    {
                        return None;
                    }
                    color[*node] = 1;
                    nodes.push(*node);
                    if self.successors[*node].is_empty()
                        && !matches!(graph.blocks[*node].end, FlowEnd::Return | FlowEnd::Throw)
                    {
                        return None;
                    }
                }
                if let Some(&target) = self.successors[*node].get(*next) {
                    *next += 1;
                    if boundary(target) {
                        continue;
                    }
                    if color[target] == 1 {
                        return None;
                    }
                    if color[target] == 0 {
                        pending.push((target, 0));
                    }
                } else {
                    color[*node] = 2;
                    pending.pop();
                }
            }
        }
        if nodes.iter().any(|&node| {
            self.predecessors[node]
                .iter()
                .any(|&source| self.reachable[source] && !lp.contains(source) && color[source] != 2)
        }) {
            return None;
        }
        nodes.sort_unstable();
        Some(nodes)
    }
}

pub(super) fn reachable_with_handlers(graph: &FlowGraph) -> Vec<BlockId> {
    let mut seen: IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
    let mut pending = vec![graph.entry];
    while let Some(node) = pending.pop() {
        if std::mem::replace(&mut seen[node], true) {
            continue;
        }
        pending.extend(graph.blocks[node].end.successors());
        for handler in crate::cfg::dispatch::effective(&graph.blocks[node].handlers)
            .filter(|_| graph.blocks[node].may_throw)
        {
            pending.push(handler.target);
        }
    }
    seen.iter_enumerated()
        .filter_map(|(node, seen)| seen.then_some(node))
        .collect()
}
