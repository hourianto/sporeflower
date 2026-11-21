//! Bound irregular control inside an exclusively owned region. Recovery of its
//! enclosing loops, branches and switches remains independent of this fallback.
use super::{BlockId, Builder, FlowEnd, FlowGraph, Stmt};
use super::{RecoveryRejection, RecoveryResult, RejectionReason};
use crate::index::IdVec;
use crate::structure::analysis::Analysis;
use std::collections::BTreeMap;

impl Builder<'_> {
    /// A region with several logical exits need not lower its enclosing method
    /// to forward jumps. Already-owned abrupt exits can be bound directly; a
    /// closed region with several normal exits is stackified only up to its
    /// own follow, preserving structured control before and after it.
    pub(super) fn bound_region(
        &mut self,
        head: BlockId,
        exits: Vec<(BlockId, BlockId)>,
        stop: Option<BlockId>,
    ) -> RecoveryResult<(Vec<Stmt>, Option<BlockId>)> {
        let mut follow = None;
        let mut bindings = Vec::new();
        let mut direct = true;
        for &(logical, target) in &exits {
            let transfer = if Some(target) == stop {
                None
            } else {
                self.transfer(target)?
            };
            if let Some(transfer) = transfer {
                bindings.push((logical, vec![transfer]));
            } else {
                if follow.is_some_and(|prior| prior != target) {
                    direct = false;
                    break;
                }
                follow = Some(target);
                bindings.push((logical, Vec::new()));
            }
        }
        if direct {
            return Ok((
                vec![Stmt::BoundRegion {
                    head,
                    exits: bindings,
                }],
                follow,
            ));
        }
        let follow = self
            .join(head)
            .or(stop)
            .ok_or_else(|| RecoveryRejection::at(head, RejectionReason::RegionBoundary))?;
        let body = self.local_region(head, Some(follow))?;
        Ok((body, Some(follow)))
    }

    /// End before the postdominating follow, or at the terminal exits of a
    /// switch arm. Only small, independently duplicable terminal blocks may
    /// have another entry. Parent ownership changes after successful recovery.
    pub(super) fn local_region(
        &mut self,
        head: BlockId,
        follow: Option<BlockId>,
    ) -> RecoveryResult<Vec<Stmt>> {
        if follow.is_some_and(|follow| {
            head == follow || self.visited[follow] || !self.analysis.postdom.dominates(follow, head)
        }) || self
            .loops
            .iter()
            .any(|scope| self.analysis.loops[scope.index].header == head)
        {
            return Err(RecoveryRejection::at(head, RejectionReason::RegionBoundary));
        }
        let mut selected: IdVec<BlockId, _> = vec![false; self.graph.blocks.len()].into();
        let mut nodes = Vec::new();
        let mut pending = vec![head];
        while let Some(node) = pending.pop() {
            if Some(node) == follow || selected[node] {
                continue;
            }
            if (node != head || follow.is_none())
                && (self.analysis.handler_entries[node]
                    || (follow.is_some() && self.visited[node])
                    || self.transfer(node)?.is_some())
            {
                return Err(RecoveryRejection::at(node, RejectionReason::RegionBoundary));
            }
            selected[node] = true;
            nodes.push(node);
            pending.extend(self.analysis.successors[node].iter().copied());
        }
        let mut duplication = self.duplicated_cost;
        for &node in &nodes {
            if node == head {
                continue;
            }
            if self.visited[node]
                || self.analysis.predecessors[node]
                    .iter()
                    .any(|&source| self.analysis.reachable[source] && !selected[source])
            {
                let block = &self.graph.blocks[node];
                // Ordinary branches keep their shared return after the switch.
                // Only bind a copy local to explicit exits of an installed region.
                let Some(cost) = block.duplication_cost.filter(|_| {
                    follow.is_none()
                        && block.handlers.is_empty()
                        && matches!(block.end, FlowEnd::Return | FlowEnd::Throw)
                        && self.analysis.predecessors[node].iter().all(|&pred| {
                            !selected[pred]
                                || matches!(self.graph.blocks[pred].end, FlowEnd::Multi(_))
                        })
                }) else {
                    return Err(RecoveryRejection::at(node, RejectionReason::RegionBoundary));
                };
                if self.visited[node] {
                    duplication += usize::from(cost);
                    if duplication > super::tail::METHOD_COST_LIMIT {
                        return Err(RecoveryRejection::at(
                            node,
                            RejectionReason::DuplicationLimit,
                        ));
                    }
                }
            }
        }
        // Keep original block IDs for source operands and semantic macros.
        // The follow is analysis-only here and expands to a logical exit;
        // its executable statements remain in the enclosing builder.
        let mut sub = FlowGraph {
            entry: head,
            blocks: self.graph.blocks.clone(),
        };
        for (node, block) in sub.blocks.iter_enumerated_mut() {
            if !selected[node] {
                block.end = FlowEnd::Unreachable;
                block.handlers.clear();
                block.may_throw = false;
                block.statements_empty = false;
            }
        }
        if let Some(follow) = follow {
            sub.blocks[follow].pc = u32::MAX;
            sub.blocks[follow].end = FlowEnd::Return;
        }
        let mut body = crate::structure::stackify::stackify(&sub, &Analysis::new(&sub))?;
        if let Some(follow) = follow {
            body = crate::structure::installation::expand(
                body,
                &BTreeMap::from([(follow, vec![Stmt::Jump(follow)])]),
            );
            body = crate::structure::bindings::bind(body, &[(follow, Vec::new())]);
        }
        let mut body = crate::structure::stackify::simplify(body);
        self.next_region = crate::structure::renumber_regions_from(&mut body, self.next_region);
        self.duplicated_cost = duplication;
        for node in nodes {
            if !self.visited[node] {
                self.mark(node);
            }
        }
        Ok(body)
    }
}
