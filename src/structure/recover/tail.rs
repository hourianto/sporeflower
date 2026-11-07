//! Give small shared continuations a source occurrence at each incoming path.
//! A switch can have several case subsets with different common consumers;
//! requiring one globally owned follow loses the switch's lexical structure.
use super::{BlockId, Builder, FlowEnd, Stmt};
use super::{RecoveryRejection, RecoveryResult, RejectionReason};

const TAIL_COST_LIMIT: usize = 48;
const METHOD_COST_LIMIT: usize = 512;

impl Builder<'_> {
    /// A linear path ends at the caller's stop, an enclosing transfer, or a
    /// return/throw. Reusing its Basic IDs preserves the original expressions
    /// and local bindings; every selected runtime path still executes it once.
    /// Plan before marking or charging anything so failed recovery is atomic.
    pub(super) fn shared_tail(
        &mut self,
        entry: BlockId,
        stop: Option<BlockId>,
    ) -> RecoveryResult<Vec<Stmt>> {
        let mut nodes = Vec::new();
        let mut body = Vec::new();
        let mut cost = 0;
        let mut copied_cost = 0;
        let mut current = entry;
        loop {
            if Some(current) == stop {
                break;
            }
            if let Some(transfer) = self.transfer(current)? {
                body.push(transfer);
                break;
            }
            let block = &self.graph.blocks[current];
            if self.analysis.handler_entries[current]
                || !block.handlers.is_empty()
                || self.analysis.loop_at[current].is_some()
                || nodes.contains(&current)
            {
                return Err(RecoveryRejection::at(current, RejectionReason::SharedTail));
            }
            let block_cost = block
                .duplication_cost
                .ok_or_else(|| RecoveryRejection::at(current, RejectionReason::SharedTail))?
                as usize;
            cost += block_cost;
            copied_cost += if self.visited[current] { block_cost } else { 0 };
            // Even a zero-cost corridor must have bounded planning work.
            if cost > TAIL_COST_LIMIT
                || nodes.len() >= TAIL_COST_LIMIT
                || self.duplicated_cost + copied_cost > METHOD_COST_LIMIT
            {
                return Err(RecoveryRejection::at(
                    current,
                    RejectionReason::DuplicationLimit,
                ));
            }
            nodes.push(current);
            body.push(Stmt::Basic(current));
            match block.end {
                FlowEnd::Goto(target) => current = target,
                FlowEnd::Return => {
                    body.push(Stmt::Return(current));
                    break;
                }
                FlowEnd::Throw => {
                    body.push(Stmt::Throw(current));
                    break;
                }
                _ => return Err(RecoveryRejection::at(current, RejectionReason::SharedTail)),
            }
        }
        for node in nodes {
            if !self.visited[node] {
                self.mark(node);
            }
        }
        self.duplicated_cost += copied_cost;
        Ok(body)
    }
}
