const B0: crate::index::BlockId = crate::index::BlockId::from_index(0);
const B1: crate::index::BlockId = crate::index::BlockId::from_index(1);
use super::*;
use crate::structure::DispatchCase;

#[test]
fn opaque_case_reentry_keeps_the_entry_inside_its_dispatcher() {
    let body = vec![Stmt::Dispatch {
        region: 3,
        entry: B0,
        exits: vec![],
        cases: vec![
            DispatchCase {
                head: B0,
                body: vec![Stmt::Basic(B0)],
                end: FlowEnd::Goto(B1),
                handlers: vec![],
            },
            DispatchCase {
                head: B1,
                body: vec![Stmt::DispatchContinue {
                    region: 3,
                    target: B0,
                }],
                end: FlowEnd::Unreachable,
                handlers: vec![],
            },
        ],
    }];
    let normalized = normalize(body);
    assert!(matches!(
        normalized.as_slice(),
        [Stmt::Dispatch { entry: B0, .. }]
    ));
}
