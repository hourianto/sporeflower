//! Bind remaining terminal and enclosing-scope loop exits to their continuation.
use super::{BlockId, Builder};
use super::{RecoveryRejection, RecoveryResult, RejectionReason};

impl Builder<'_> {
    pub(super) fn loop_follow(
        &self,
        index: usize,
    ) -> RecoveryResult<(Option<BlockId>, Vec<BlockId>)> {
        let lp = &self.analysis.loops[index];
        if lp.exits.len() <= 1 {
            return Ok((lp.exits.first().copied(), vec![]));
        }
        let preferred = self.analysis.loop_guard(self.graph, index);
        let follows = preferred
            .into_iter()
            .chain(self.analysis.common_postdom(lp.exits.iter().copied()))
            .chain(lp.exits.iter().rev().copied());
        let mut tried = std::collections::BTreeSet::new();
        for follow in follows {
            if lp.contains(follow) || self.visited[follow] || !tried.insert(follow) {
                continue;
            }
            if let Some(nodes) = self.analysis.exit_region(self.graph, index, |node| {
                node == follow || self.transfer(node).is_ok_and(|exit| exit.is_some())
            }) && nodes.iter().all(|&node| !self.visited[node])
            {
                return Ok((Some(follow), nodes));
            }
        }
        Err(RecoveryRejection::at(
            lp.header,
            RejectionReason::LoopExit { header: lp.header },
        ))
    }
}
