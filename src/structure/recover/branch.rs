//! Recover branch continuations in their lexical loop/switch context.
use super::RecoveryResult;
use super::{BlockId, Builder, Predicate, Stmt};

impl Builder<'_> {
    pub(super) fn branch_arms(
        &mut self,
        head: BlockId,
        condition: &Predicate,
        then_target: BlockId,
        else_target: BlockId,
        stop: Option<BlockId>,
        other_cases: &[BlockId],
    ) -> RecoveryResult<(Vec<Stmt>, Vec<Stmt>, Option<BlockId>)> {
        let join = self.join(head).or(stop);
        let contextual = if join == Some(then_target) || join == Some(else_target) {
            join
        } else {
            self.shared_follow(
                &condition.heads(),
                &[then_target, else_target],
                stop.or(join),
            )
        };
        if contextual.is_some() && contextual != join {
            let checkpoint = self.checkpoint();
            let arms = self
                .arm(then_target, contextual, other_cases)
                .and_then(|yes| {
                    self.arm(else_target, contextual, other_cases)
                        .map(|no| (yes, no, contextual))
                });
            if arms.is_ok() {
                return arms;
            }
            self.rollback(checkpoint);
        }
        Ok((
            self.arm(then_target, join, other_cases)?,
            self.arm(else_target, join, other_cases)?,
            join,
        ))
    }
}
