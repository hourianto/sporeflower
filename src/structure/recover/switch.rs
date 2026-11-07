//! Keep a switch with shared case tails local when ordinary arm recovery fails.
use super::RecoveryResult;
use std::collections::BTreeMap;

use super::{BlockId, Builder, Stmt, SwitchScope};
use crate::structure::{SwitchCase, can_complete};

impl Builder<'_> {
    pub(super) fn switch(
        &mut self,
        head: BlockId,
        cases: Vec<(i32, BlockId)>,
        default: BlockId,
        stop: Option<BlockId>,
    ) -> RecoveryResult<(Vec<Stmt>, Option<BlockId>)> {
        let follow = self.join(head).or(stop);
        let checkpoint = self.checkpoint();
        let mut entries: Vec<_> = cases.iter().map(|&(_, target)| target).collect();
        entries.push(default);
        entries.sort_unstable();
        entries.dedup();
        if let Some(shared) = self.shared_follow(&[head], &entries, follow)
            && Some(shared) != follow
        {
            if let Ok(body) = self.switch_arms(head, &cases, default, Some(shared)) {
                return Ok((body, Some(shared)));
            }
            self.rollback(checkpoint);
        }
        let rejection = match self.switch_arms(head, &cases, default, follow) {
            Ok(body) => return Ok((body, follow)),
            Err(rejection) => rejection,
        };
        // Failed arm recovery owns nothing. Restore only the nodes it visited,
        // rather than copying the whole method's state for every switch.
        self.rollback(checkpoint);
        let follow = follow.ok_or(rejection)?;
        let body = self.local_region(head, follow)?;
        Ok((body, Some(follow)))
    }

    fn switch_arms(
        &mut self,
        head: BlockId,
        cases: &[(i32, BlockId)],
        default: BlockId,
        follow: Option<BlockId>,
    ) -> RecoveryResult<Vec<Stmt>> {
        let region = self.region();
        let mut grouped: BTreeMap<BlockId, SwitchCase> = BTreeMap::new();
        for &(key, target) in cases {
            grouped
                .entry(target)
                .or_insert_with(|| SwitchCase {
                    keys: Vec::new(),
                    default: false,
                    body: Vec::new(),
                })
                .keys
                .push(key);
        }
        grouped
            .entry(default)
            .or_insert_with(|| SwitchCase {
                keys: Vec::new(),
                default: false,
                body: Vec::new(),
            })
            .default = true;
        let targets = self.case_order(grouped.keys().copied().collect(), follow)?;
        let mut structured_cases = Vec::new();
        self.switches.push(SwitchScope { region, follow });
        for (index, &target) in targets.iter().enumerate() {
            let mut case = grouped.remove(&target).unwrap();
            let next_case = targets.get(index + 1).copied();
            case.body = self.arm(target, next_case.or(follow), &targets)?;
            if next_case.is_none() && can_complete(&case.body) {
                case.body.push(Stmt::Break { target: region });
            }
            structured_cases.push(case);
        }
        self.switches.pop();
        Ok(vec![
            Stmt::Basic(head),
            Stmt::Switch {
                region,
                head,
                cases: structured_cases,
            },
        ])
    }
}
