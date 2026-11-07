//! Prove an iteration boundary and optional shared switch follow before rendering.
use super::{BlockId, Builder, FlowEnd, Stmt};
use crate::index::IdVec;
use crate::ir::{MethodIr, Statement, Terminator, local_effects};
use std::collections::BTreeSet;

#[cfg(test)]
#[path = "continuation_tests.rs"]
mod tests;

pub(crate) fn is_loop_update(ir: &MethodIr, block: BlockId, may_throw: bool) -> bool {
    let block = &ir.blocks[block];
    if may_throw || !matches!(block.terminator, Terminator::Goto(_)) {
        return false;
    }
    let [statement @ Statement::Assign { local, .. }] = block.statements.as_slice() else {
        return false;
    };
    if ir.locals[*local].is_this {
        return false;
    }
    let effect = local_effects::statement(ir, statement, false);
    let mut reads = BTreeSet::new();
    effect.reads_before_writes(&mut reads, &mut BTreeSet::new());
    let mut writes = Vec::new();
    effect.writes(&mut |local| writes.push(local));
    reads.contains(local) && writes == [*local]
}

impl Builder<'_> {
    pub(super) fn owned_continuation(&self, index: usize) -> Option<BlockId> {
        let lp = &self.analysis.loops[index];
        let mut backedges = self.analysis.predecessors[lp.header]
            .iter()
            .copied()
            .filter(|&source| lp.contains(source));
        let latch = backedges.next()?;
        if backedges.next().is_some()
            || latch == lp.header
            || self.visited[latch]
            || self.analysis.handler_entries[latch]
            || !self.graph.blocks[latch].is_loop_update
            || !self.graph.blocks[latch].handlers.is_empty()
            || !matches!(self.graph.blocks[latch].end, FlowEnd::Goto(target) if target == lp.header)
            || self.analysis.predecessors[latch]
                .iter()
                .any(|&source| self.analysis.reachable[source] && !lp.contains(source))
        {
            return None;
        }
        Some(latch)
    }

    /// Find the first continuation shared by independent arms in the current
    /// lexical context. Method-wide postdominators include returns and loop
    /// backedges; neither is a normal completion of the branch or switch.
    ///
    /// Propagate an arm identity through the graph, joining distinct identities
    /// to Shared. Every node changes at most twice, including inside natural
    /// loops. Shared entries can themselves reconverge: a switch may have
    /// several fallthrough case groups before its ordinary continuation. Their
    /// common postdominator bounds that shared suffix. If a fallthrough group
    /// can return early, the last shared entry instead bounds its normal paths.
    /// Disjoint arm recovery still proves that everything before the selected
    /// continuation has a Java representation.
    pub(super) fn shared_follow(
        &self,
        heads: &[BlockId],
        entries: &[BlockId],
        stop: Option<BlockId>,
    ) -> Option<BlockId> {
        const SHARED: usize = usize::MAX;
        let mut owners: IdVec<BlockId, _> = vec![0usize; self.graph.blocks.len()].into();
        let mut pending = std::collections::VecDeque::new();
        let normal_stop = stop.filter(|&target| {
            matches!(heads, [head] if matches!(self.graph.blocks[*head].end, FlowEnd::Switch { .. }))
                && matches!(self.transfer(target), Ok(Some(Stmt::Break { target: region }))
                if self.switches.iter().any(|scope| scope.region == region))
        });
        for (index, &entry) in entries.iter().enumerate() {
            if Some(entry) != normal_stop && self.transfer(entry).ok()?.is_some() {
                continue;
            }
            if owners[entry] == 0 {
                owners[entry] = index + 1;
                pending.push_back(entry);
            }
        }
        while let Some(node) = pending.pop_front() {
            if self.visited[node] || self.analysis.handler_entries[node] {
                return None;
            }
            if Some(node) == stop {
                continue;
            }
            for &target in &self.analysis.successors[node] {
                // The caller's stop is a normal endpoint even when an outer
                // switch owns it as a break. Only a nested switch introduces
                // another break scope at that same endpoint; branches and
                // loop transfers still distinguish early exits from normal
                // arm completion.
                if Some(target) != normal_stop && self.transfer(target).ok()?.is_some() {
                    continue;
                }
                let next = if owners[target] == 0 || owners[target] == owners[node] {
                    owners[node]
                } else {
                    SHARED
                };
                if next != owners[target] {
                    owners[target] = next;
                    pending.push_back(target);
                }
            }
        }
        let frontier: Vec<_> = owners
            .iter_enumerated()
            .filter_map(|(node, &owner)| {
                (owner == SHARED
                    && (entries.contains(&node)
                        || self.analysis.predecessors[node]
                            .iter()
                            .any(|&source| owners[source] != 0 && owners[source] != SHARED)))
                .then_some(node)
            })
            .collect();
        let follow = self
            .analysis
            .common_postdom(frontier.iter().copied())
            .or_else(|| self.last_shared_frontier(&owners, &frontier, stop))?;
        // Postdominators describe the whole method. A join beyond the lexical
        // stop or an enclosing abrupt transfer does not belong to this search.
        if owners[follow] != SHARED {
            return None;
        }
        // The shared suffix is not owned by any arm. Everything before it
        // must have entered through this predicate/selector, never through a
        // sibling case or handler. Stop at enclosing abrupt transfers too.
        let mut owned: crate::index::IdVec<BlockId, _> =
            vec![false; self.graph.blocks.len()].into();
        let mut pending = entries.to_vec();
        while let Some(node) = pending.pop() {
            if node == follow || owned[node] || self.transfer(node).ok()?.is_some() {
                continue;
            }
            if self.visited[node] || self.analysis.handler_entries[node] || Some(node) == stop {
                return None;
            }
            owned[node] = true;
            pending.extend(self.analysis.successors[node].iter().copied());
        }
        owned
            .iter_enumerated()
            .all(|(node, &selected)| {
                !selected
                    || self.analysis.predecessors[node].iter().all(|&source| {
                        !self.analysis.reachable[source] || owned[source] || heads.contains(&source)
                    })
            })
            .then_some(follow)
    }

    /// A returning fallthrough group need not postdominate its predecessors.
    /// Its normal continuation is nevertheless unambiguous when all shared
    /// entries reach one final shared entry and that entry reaches no earlier
    /// one. Find a candidate in DFS finish order, then prove both directions.
    /// These walks are linear and stop at the same lexical boundary as the
    /// ownership propagation; loops between competing frontiers are rejected.
    fn last_shared_frontier(
        &self,
        owners: &IdVec<BlockId, usize>,
        frontier: &[BlockId],
        stop: Option<BlockId>,
    ) -> Option<BlockId> {
        let first = *frontier.first()?;
        let mut is_frontier: crate::index::IdVec<BlockId, _> = vec![false; owners.len()].into();
        for &node in frontier {
            is_frontier[node] = true;
        }
        let mut seen: IdVec<BlockId, _> = vec![false; owners.len()].into();
        seen[first] = true;
        let mut stack = vec![(first, 0)];
        let candidate = loop {
            let (node, edge) = stack.last_mut()?;
            let successors = &self.analysis.successors[*node];
            if Some(*node) == stop || *edge == successors.len() {
                let (node, _) = stack.pop().unwrap();
                if is_frontier[node] {
                    break node;
                }
            } else {
                let target = successors[*edge];
                *edge += 1;
                if owners[target] == usize::MAX && !std::mem::replace(&mut seen[target], true) {
                    stack.push((target, 0));
                }
            }
        };

        seen.fill(false);
        let mut pending = vec![candidate];
        while let Some(node) = pending.pop() {
            if std::mem::replace(&mut seen[node], true) {
                continue;
            }
            pending.extend(
                self.analysis.predecessors[node]
                    .iter()
                    .copied()
                    .filter(|&source| owners[source] == usize::MAX && Some(source) != stop),
            );
        }
        if frontier.iter().any(|&node| !seen[node]) {
            return None;
        }

        seen.fill(false);
        let mut pending = vec![candidate];
        while let Some(node) = pending.pop() {
            if std::mem::replace(&mut seen[node], true) {
                continue;
            }
            if node != candidate && is_frontier[node] {
                return None;
            }
            if Some(node) != stop {
                pending.extend(
                    self.analysis.successors[node]
                        .iter()
                        .copied()
                        .filter(|&target| owners[target] == usize::MAX),
                );
            }
        }
        Some(candidate)
    }
}
