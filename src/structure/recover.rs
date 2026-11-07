mod branch;
mod condition;
mod continuation;
mod loops;
mod region;
mod switch;
mod tail;
#[cfg(test)]
mod tests;
pub(super) use condition::Decisions;
pub(super) use continuation::is_loop_update;

use super::analysis::Analysis;
use super::attempt::{RecoveryRejection, RecoveryResult, RejectionReason};
use super::{BlockId, FlowEnd, FlowGraph, Predicate, RegionId, Stmt};
use crate::index::IdVec;

#[derive(Clone)]
struct LoopScope {
    index: usize,
    region: RegionId,
    follow: Option<BlockId>,
    /// Owned acyclic exit paths, ending at the follow or a terminal transfer.
    terminal_nodes: Vec<BlockId>,
    continuation: Option<BlockId>,
}

#[derive(Clone, Copy)]
struct SwitchScope {
    region: RegionId,
    follow: Option<BlockId>,
}

struct Builder<'a> {
    graph: &'a FlowGraph,
    analysis: &'a Analysis,
    visited: IdVec<BlockId, bool>,
    visited_order: Vec<BlockId>,
    loops: Vec<LoopScope>,
    switches: Vec<SwitchScope>,
    next_region: usize,
    duplicated_cost: usize,
    depth: usize,
}

#[derive(Clone, Copy)]
struct Checkpoint {
    visited: usize,
    loops: usize,
    switches: usize,
    next_region: usize,
    duplicated_cost: usize,
}

pub(super) fn recover(graph: &FlowGraph, analysis: &Analysis) -> RecoveryResult<Vec<Stmt>> {
    if let Some((id, _)) = graph.blocks.iter_enumerated().find(|&(id, block)| {
        analysis.reachable[id] && block.may_throw && !block.handlers.is_empty()
    }) {
        return Err(RecoveryRejection::at(id, RejectionReason::ProtectedBlock));
    }
    let mut builder = Builder::new(graph, analysis);
    let body = builder.sequence(graph.entry, None, &[])?;
    if let Some((id, _)) = analysis
        .reachable
        .iter_enumerated()
        .find(|&(id, &reachable)| reachable && !builder.visited[id])
    {
        return Err(RecoveryRejection::at(id, RejectionReason::RegionBoundary));
    }
    Ok(body)
}

impl<'a> Builder<'a> {
    fn new(graph: &'a FlowGraph, analysis: &'a Analysis) -> Self {
        Self {
            graph,
            analysis,
            visited: vec![false; graph.blocks.len()].into(),
            visited_order: Vec::new(),
            loops: Vec::new(),
            switches: Vec::new(),
            next_region: 0,
            duplicated_cost: 0,
            depth: 0,
        }
    }

    fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            visited: self.visited_order.len(),
            loops: self.loops.len(),
            switches: self.switches.len(),
            next_region: self.next_region,
            duplicated_cost: self.duplicated_cost,
        }
    }

    fn rollback(&mut self, checkpoint: Checkpoint) {
        for node in self.visited_order.drain(checkpoint.visited..) {
            self.visited[node] = false;
        }
        self.loops.truncate(checkpoint.loops);
        self.switches.truncate(checkpoint.switches);
        self.next_region = checkpoint.next_region;
        self.duplicated_cost = checkpoint.duplicated_cost;
    }

    fn mark(&mut self, block: BlockId) {
        debug_assert!(!self.visited[block]);
        self.visited[block] = true;
        self.visited_order.push(block);
    }

    fn region(&mut self) -> RegionId {
        let result = self.next_region;
        self.next_region += 1;
        result
    }

    fn transfer(&self, target: BlockId) -> RecoveryResult<Option<Stmt>> {
        for scope in self.loops.iter().rev() {
            let lp = &self.analysis.loops[scope.index];
            if target == lp.header {
                return Ok(Some(Stmt::Continue { header: target }));
            }
            if scope.continuation == Some(target) {
                return Ok(Some(Stmt::Continue { header: lp.header }));
            }
            if scope.follow == Some(target) {
                return Ok(Some(Stmt::Break {
                    target: scope.region,
                }));
            }
        }
        for scope in self.switches.iter().rev() {
            if scope.follow == Some(target) {
                return Ok(Some(Stmt::Break {
                    target: scope.region,
                }));
            }
        }
        if let Some(scope) = self.loops.last()
            && !self.analysis.loops[scope.index].contains(target)
            && scope.terminal_nodes.binary_search(&target).is_err()
        {
            return Err(RecoveryRejection::at(
                target,
                RejectionReason::LoopExit {
                    header: self.analysis.loops[scope.index].header,
                },
            ));
        }
        Ok(None)
    }

    fn arm(
        &mut self,
        target: BlockId,
        stop: Option<BlockId>,
        other_cases: &[BlockId],
    ) -> RecoveryResult<Vec<Stmt>> {
        if Some(target) == stop {
            return Ok(Vec::new());
        }
        if let Some(exit) = self.transfer(target)? {
            return Ok(vec![exit]);
        }
        self.sequence(target, stop, other_cases)
    }

    fn sequence(
        &mut self,
        entry: BlockId,
        stop: Option<BlockId>,
        other_cases: &[BlockId],
    ) -> RecoveryResult<Vec<Stmt>> {
        if self.depth >= 384 {
            return Err(RecoveryRejection::at(entry, RejectionReason::NestingLimit));
        }
        self.depth += 1;
        let result = self.sequence_inner(entry, stop, other_cases);
        self.depth -= 1;
        result
    }

    fn sequence_inner(
        &mut self,
        entry: BlockId,
        stop: Option<BlockId>,
        other_cases: &[BlockId],
    ) -> RecoveryResult<Vec<Stmt>> {
        let mut body = Vec::new();
        let mut current = entry;
        loop {
            if Some(current) == stop {
                break;
            }
            if self.visited[current] || current != entry && other_cases.contains(&current) {
                body.extend(self.shared_tail(current, stop)?);
                break;
            }

            if let Some(index) = self.analysis.loop_at[current]
                && !self.loops.iter().any(|scope| scope.index == index)
            {
                let (follow, terminal_nodes) = self.loop_follow(index)?;
                let continuation = self.owned_continuation(index);
                let region = self.region();
                self.loops.push(LoopScope {
                    index,
                    region,
                    follow,
                    terminal_nodes,
                    continuation,
                });
                let mut loop_body = self.sequence(current, None, &[])?;
                self.loops.pop();
                if let Some(continuation) = continuation {
                    self.mark(continuation);
                }
                if matches!(loop_body.last(), Some(Stmt::Continue { header }) if *header == current)
                {
                    loop_body.pop();
                }
                body.push(Stmt::Loop {
                    region,
                    header: current,
                    body: loop_body,
                    continuation,
                });
                if let Some(follow) = follow {
                    if Some(follow) == stop {
                        break;
                    }
                    if let Some(exit) = self.transfer(follow)? {
                        body.push(exit);
                        break;
                    }
                    current = follow;
                    continue;
                }
                break;
            }

            self.mark(current);
            body.push(Stmt::Basic(current));
            match self.graph.blocks[current].end.clone() {
                FlowEnd::Multi(exits) => {
                    // A semantic region already owns its effects and handler
                    // scopes. Bind only control transfers here; moving its
                    // successors into the region would change catch coverage.
                    body.pop();
                    let (region, follow) = self.bound_region(current, exits, stop)?;
                    body.extend(region);
                    if let Some(follow) = follow {
                        if Some(follow) == stop {
                            break;
                        }
                        if let Some(exit) = self.transfer(follow)? {
                            body.push(exit);
                            break;
                        }
                        current = follow;
                    } else {
                        break;
                    }
                }
                FlowEnd::Return => {
                    body.push(Stmt::Return(current));
                    break;
                }
                FlowEnd::Throw => {
                    body.push(Stmt::Throw(current));
                    break;
                }
                FlowEnd::Unreachable => {
                    body.push(Stmt::Unreachable(current));
                    break;
                }
                FlowEnd::Goto(target) => {
                    if Some(target) == stop {
                        break;
                    }
                    if let Some(exit) = self.transfer(target)? {
                        body.push(exit);
                        break;
                    }
                    current = target;
                }
                FlowEnd::Branch {
                    then_target,
                    else_target,
                } => {
                    if then_target == else_target {
                        // The transfer is redundant, but evaluating its
                        // operands is not: a call, array read or inline local
                        // assignment can still throw or change state here.
                        body.push(Stmt::If {
                            condition: Predicate::Atom {
                                head: current,
                                negated: false,
                            },
                            then_body: Vec::new(),
                            else_body: Vec::new(),
                        });
                        if Some(then_target) == stop {
                            break;
                        }
                        if let Some(exit) = self.transfer(then_target)? {
                            body.push(exit);
                            break;
                        }
                        current = then_target;
                        continue;
                    }
                    let checkpoint = self.checkpoint();
                    let condition = Predicate::Atom {
                        head: current,
                        negated: false,
                    };
                    let then_exit = if Some(then_target) == stop {
                        None
                    } else {
                        self.transfer(then_target)?
                    };
                    let else_exit = if Some(else_target) == stop {
                        None
                    } else {
                        self.transfer(else_target)?
                    };
                    if then_exit.is_some() || else_exit.is_some() {
                        match (then_exit, else_exit) {
                            (Some(a), Some(b)) => {
                                body.push(Stmt::If {
                                    condition,
                                    then_body: vec![a],
                                    else_body: vec![b],
                                });
                                break;
                            }
                            (Some(exit), None) => {
                                body.push(Stmt::If {
                                    condition,
                                    then_body: vec![exit],
                                    else_body: Vec::new(),
                                });
                                current = else_target;
                            }
                            (None, Some(exit)) => {
                                body.push(Stmt::If {
                                    condition: condition.negated(),
                                    then_body: vec![exit],
                                    else_body: Vec::new(),
                                });
                                current = then_target;
                            }
                            _ => unreachable!(),
                        }
                        continue;
                    }
                    let arms = self.branch_arms(
                        current,
                        &condition,
                        then_target,
                        else_target,
                        stop,
                        other_cases,
                    );
                    let (then_body, else_body, join) = match arms {
                        Ok(arms) => arms,
                        Err(rejection) => {
                            // An internal shared tail can prevent disjoint arm
                            // recovery without invalidating its enclosing case
                            // or loop. Keep only this closed region lexical.
                            self.rollback(checkpoint);
                            let follow = self.join(current).or(stop).ok_or(rejection)?;
                            let region = self.local_region(current, follow)?;
                            body.pop();
                            body.extend(region);
                            if Some(follow) == stop {
                                break;
                            }
                            if let Some(exit) = self.transfer(follow)? {
                                body.push(exit);
                                break;
                            }
                            current = follow;
                            continue;
                        }
                    };
                    body.push(super::predicates::fold_if(condition, then_body, else_body));
                    if let Some(join) = join {
                        if Some(join) == stop {
                            break;
                        }
                        if let Some(exit) = self.transfer(join)? {
                            body.push(exit);
                            break;
                        }
                        current = join;
                    } else {
                        break;
                    }
                }
                FlowEnd::Switch { cases, default } => {
                    // The switch helper owns its head's Basic statement too,
                    // allowing a local fallback to return one complete region.
                    body.pop();
                    let (switch_body, join) = self.switch(current, cases, default, stop)?;
                    body.extend(switch_body);
                    if let Some(join) = join {
                        if Some(join) == stop {
                            break;
                        }
                        if let Some(exit) = self.transfer(join)? {
                            body.push(exit);
                            break;
                        }
                        current = join;
                    } else {
                        break;
                    }
                }
            }
        }
        Ok(body)
    }

    fn join(&self, head: BlockId) -> Option<BlockId> {
        self.analysis.postdom.idom[head].filter(|&join| {
            self.graph.blocks.contains_id(join)
                && join != head
                && self.loops.last().is_none_or(|scope| {
                    let lp = &self.analysis.loops[scope.index];
                    (lp.contains(join) || scope.terminal_nodes.binary_search(&join).is_ok())
                        && join != lp.header
                })
        })
    }

    fn case_order(
        &self,
        targets: Vec<BlockId>,
        join: Option<BlockId>,
    ) -> RecoveryResult<Vec<BlockId>> {
        let mut edges = vec![Vec::new(); targets.len()];
        let mut indegree = vec![0; targets.len()];
        let mut target_index: IdVec<BlockId, _> = vec![usize::MAX; self.graph.blocks.len()].into();
        for (index, &target) in targets.iter().enumerate() {
            target_index[target] = index;
        }
        let mut seen: IdVec<BlockId, _> = vec![0; self.graph.blocks.len()].into();
        for (index, &entry) in targets.iter().enumerate() {
            let mut pending = vec![entry];
            while let Some(node) = pending.pop() {
                if Some(node) == join || seen[node] == index + 1 {
                    continue;
                }
                seen[node] = index + 1;
                if node != entry && target_index[node] != usize::MAX {
                    edges[index].push(target_index[node]);
                    continue;
                }
                for &target in &self.analysis.successors[node] {
                    if self
                        .transfer(target)
                        .is_ok_and(|transfer| transfer.is_none())
                    {
                        pending.push(target);
                    }
                }
            }
            edges[index].sort_unstable();
            edges[index].dedup();
            for &target in &edges[index] {
                indegree[target] += 1;
            }
        }
        let mut ready = std::collections::BTreeSet::new();
        for (index, &degree) in indegree.iter().enumerate() {
            if degree == 0 {
                ready.insert((self.graph.blocks[targets[index]].pc, targets[index], index));
            }
        }
        let mut ordered = Vec::new();
        while let Some((_, target, index)) = ready.pop_first() {
            ordered.push(target);
            for &next in &edges[index] {
                indegree[next] -= 1;
                if indegree[next] == 0 {
                    ready.insert((self.graph.blocks[targets[next]].pc, targets[next], next));
                }
            }
        }
        if ordered.len() == targets.len() {
            Ok(ordered)
        } else {
            let blocked = targets
                .iter()
                .zip(indegree)
                .find_map(|(&target, degree)| (degree != 0).then_some(target))
                .unwrap();
            Err(RecoveryRejection::at(blocked, RejectionReason::CyclicOrder))
        }
    }
}
