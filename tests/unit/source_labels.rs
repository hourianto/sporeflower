use super::*;
use crate::index::test_ids::*;

#[test]
fn repeated_bytecode_loops_resolve_continues_in_each_lexical_copy() {
    let copy = |region| Stmt::Loop {
        region,
        header: B10,
        continuation: None,
        body: vec![Stmt::Loop {
            region: region + 1,
            header: B20,
            continuation: None,
            body: vec![Stmt::Continue { header: B10 }],
        }],
    };
    assert_eq!(
        collect_labels(&[copy(1), copy(3)]),
        HashSet::from_iter([1, 3])
    );
}

#[test]
fn nested_copies_restore_the_enclosing_loop_header() {
    let continue_from = |region| Stmt::Loop {
        region,
        header: B20,
        continuation: None,
        body: vec![Stmt::Continue { header: B10 }],
    };
    let body = vec![Stmt::Loop {
        region: 1,
        header: B10,
        continuation: None,
        body: vec![
            Stmt::Loop {
                region: 2,
                header: B10,
                continuation: None,
                body: vec![continue_from(3)],
            },
            continue_from(4),
        ],
    }];
    assert_eq!(collect_labels(&body), HashSet::from_iter([1, 2]));
}

#[test]
fn dispatcher_syntax_requires_labels_on_enclosing_transfers() {
    let body = vec![Stmt::Loop {
        region: 1,
        header: B10,
        continuation: None,
        body: vec![Stmt::Dispatch {
            region: 2,
            entry: B0,
            exits: Vec::new(),
            cases: vec![DispatchCase {
                head: B0,
                body: vec![Stmt::Continue { header: B10 }, Stmt::Break { target: 1 }],
                end: FlowEnd::Unreachable,
                handlers: Vec::new(),
            }],
        }],
    }];
    assert_eq!(collect_labels(&body), HashSet::from_iter([1]));
}

#[test]
fn plain_labels_do_not_intercept_implicit_loop_breaks() {
    let body = vec![Stmt::Loop {
        region: 1,
        header: B10,
        continuation: None,
        body: vec![Stmt::Label {
            region: 2,
            body: vec![Stmt::Break { target: 1 }],
        }],
    }];
    assert_eq!(collect_labels(&body), HashSet::from_iter([2]));
}

#[test]
fn nested_switch_exit_names_only_the_outer_switch() {
    let body = vec![Stmt::Switch {
        region: 1,
        head: B0,
        cases: vec![crate::structure::SwitchCase {
            keys: vec![0],
            default: false,
            body: vec![
                Stmt::Switch {
                    region: 2,
                    head: B10,
                    cases: vec![crate::structure::SwitchCase {
                        keys: vec![],
                        default: true,
                        body: vec![Stmt::Break { target: 1 }],
                    }],
                },
                Stmt::Break { target: 1 },
            ],
        }],
    }];
    assert_eq!(collect_labels(&body), HashSet::from_iter([1]));
}
