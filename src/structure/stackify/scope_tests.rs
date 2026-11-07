use super::*;
const B0: crate::index::BlockId = crate::index::BlockId::from_index(0);
const B1: crate::index::BlockId = crate::index::BlockId::from_index(1);
const B2: crate::index::BlockId = crate::index::BlockId::from_index(2);
const B3: crate::index::BlockId = crate::index::BlockId::from_index(3);
const B4: crate::index::BlockId = crate::index::BlockId::from_index(4);
const B5: crate::index::BlockId = crate::index::BlockId::from_index(5);
const B6: crate::index::BlockId = crate::index::BlockId::from_index(6);
const B10: crate::index::BlockId = crate::index::BlockId::from_index(10);
const B40: crate::index::BlockId = crate::index::BlockId::from_index(40);
const B41: crate::index::BlockId = crate::index::BlockId::from_index(41);
const V0: crate::index::ValueId = crate::index::ValueId::from_index(0);
use crate::structure::{Catch, Predicate, SwitchCase};

const LABEL: usize = 1000;
const SWITCH: usize = 2000;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Exit {
    Break(usize),
    Return(usize),
    Throw(usize),
}

fn test(predicate: &Predicate, bits: u32, trace: &mut Vec<usize>) -> bool {
    match predicate {
        Predicate::Atom { head, negated } => {
            trace.push(100 + head.index());
            ((bits >> head.index()) & 1 != 0) != *negated
        }
        Predicate::And(terms) => terms.iter().all(|term| test(term, bits, trace)),
        Predicate::Or(terms) => terms.iter().any(|term| test(term, bits, trace)),
        Predicate::Select { condition, yes, no } => {
            let selected = if test(condition, bits, trace) {
                yes
            } else {
                no
            };
            test(selected, bits, trace)
        }
    }
}

fn execute(body: &[Stmt], bits: u32, fail: Option<usize>, trace: &mut Vec<usize>) -> Option<Exit> {
    for statement in body {
        let exit = match statement {
            Stmt::Basic(block) => {
                trace.push(block.index());
                (fail == Some(block.index())).then_some(Exit::Throw(block.index()))
            }
            Stmt::Break { target } => Some(Exit::Break(*target)),
            Stmt::Return(block) => Some(Exit::Return(block.index())),
            Stmt::Throw(block) => Some(Exit::Throw(block.index())),
            Stmt::If {
                condition,
                then_body,
                else_body,
            } => execute(
                if test(condition, bits, trace) {
                    then_body
                } else {
                    else_body
                },
                bits,
                fail,
                trace,
            ),
            Stmt::Label { region, body } => {
                execute(body, bits, fail, trace).filter(|exit| *exit != Exit::Break(*region))
            }
            Stmt::Switch {
                region,
                head,
                cases,
            } => {
                trace.push(500 + head.index());
                let selector = ((bits >> 2) & 3) as i32 - 1;
                let start = cases
                    .iter()
                    .position(|case| case.keys.contains(&selector))
                    .or_else(|| cases.iter().position(|case| case.default));
                let mut result = None;
                if let Some(start) = start {
                    for case in &cases[start..] {
                        result = execute(&case.body, bits, fail, trace);
                        if result.is_some() {
                            break;
                        }
                    }
                }
                result.filter(|exit| *exit != Exit::Break(*region))
            }
            Stmt::Try { body, catches } => {
                let exit = execute(body, bits, fail, trace);
                if let Some(Exit::Throw(value)) = exit {
                    let caught = catches.iter().find(|catch| {
                        catch
                            .catch_type
                            .as_deref()
                            .is_none_or(|kind| kind == if value % 2 == 0 { "Even" } else { "Odd" })
                    });
                    if let Some(catch) = caught {
                        trace.push(200 + catch.handler.index());
                        execute(&catch.body, bits, fail, trace)
                    } else {
                        exit
                    }
                } else {
                    exit
                }
            }
            Stmt::TryFinally { body, cleanup, .. } => {
                let pending = execute(body, bits, fail, trace);
                execute(cleanup, bits, fail, trace).or(pending)
            }
            Stmt::Synchronized { monitor, body } => {
                trace.push(300 + monitor.index());
                let pending = execute(body, bits, fail, trace);
                trace.push(400 + monitor.index());
                pending
            }
            _ => panic!("unexpected statement {statement:?}"),
        };
        if exit.is_some() {
            return exit;
        }
    }
    None
}

fn decision(head: usize, then_body: Vec<Stmt>) -> Stmt {
    Stmt::If {
        condition: Predicate::Atom {
            head: BlockId::from_index(head),
            negated: false,
        },
        then_body,
        else_body: vec![],
    }
}

fn exit() -> Stmt {
    Stmt::Break { target: LABEL }
}

fn catch(kind: Option<&str>, handler: usize, body: Vec<Stmt>) -> Catch {
    Catch {
        catch_type: kind.map(str::to_owned),
        handler: BlockId::from_index(handler),
        body,
    }
}

fn protected() -> Stmt {
    Stmt::Try {
        body: vec![
            Stmt::Basic(B1),
            decision(0, vec![exit()]),
            Stmt::Basic(B2),
            Stmt::Return(B40),
        ],
        catches: vec![
            catch(
                Some("Odd"),
                0,
                vec![Stmt::Basic(B3), decision(1, vec![exit()]), Stmt::Throw(B41)],
            ),
            catch(None, 1, vec![Stmt::Basic(B4), exit()]),
        ],
    }
}

fn check(body: Vec<Stmt>, removed: bool) {
    let original = vec![Stmt::Label {
        region: LABEL,
        body,
    }];
    let recovered = simplify(original.clone());
    assert_eq!(
        !matches!(recovered.first(), Some(Stmt::Label { region: LABEL, .. })),
        removed,
        "{recovered:#?}"
    );
    for bits in 0..32 {
        for fail in std::iter::once(None).chain((1..=8).map(Some)) {
            let mut before = vec![];
            let mut after = vec![];
            let expected = execute(&original, bits, fail, &mut before);
            let actual = execute(&recovered, bits, fail, &mut after);
            assert_eq!(actual, expected, "bits={bits}, fail={fail:?}");
            assert_eq!(after, before, "bits={bits}, fail={fail:?}");
        }
    }
}

#[test]
fn terminal_scopes_preserve_handlers_cleanup_and_abrupt_completions() {
    check(vec![protected()], true);
    check(
        vec![Stmt::TryFinally {
            body: vec![protected()],
            cleanup: vec![Stmt::Basic(B5)],
            handler: B10,
        }],
        true,
    );
    check(
        vec![Stmt::Synchronized {
            monitor: V0,
            body: vec![protected()],
        }],
        true,
    );
    check(
        vec![Stmt::Label {
            region: LABEL + 1,
            body: vec![
                decision(2, vec![Stmt::Break { target: LABEL + 1 }]),
                protected(),
            ],
        }],
        true,
    );
}

#[test]
fn terminal_labels_share_one_continuation_across_protected_scopes() {
    check(
        vec![Stmt::Label {
            region: LABEL + 1,
            body: vec![Stmt::Label {
                region: LABEL + 2,
                body: vec![Stmt::TryFinally {
                    body: vec![
                        Stmt::Label {
                            region: LABEL + 3,
                            body: vec![
                                Stmt::Try {
                                    body: vec![
                                        Stmt::Basic(B1),
                                        decision(
                                            0,
                                            vec![
                                                Stmt::Basic(B2),
                                                Stmt::Break { target: LABEL + 3 },
                                            ],
                                        ),
                                    ],
                                    catches: vec![catch(
                                        None,
                                        0,
                                        vec![Stmt::Basic(B3), Stmt::Break { target: LABEL + 1 }],
                                    )],
                                },
                                Stmt::Break { target: LABEL + 2 },
                            ],
                        },
                        exit(),
                    ],
                    cleanup: vec![Stmt::Basic(B4)],
                    handler: B10,
                }],
            }],
        }],
        true,
    );
}

#[test]
fn continuation_effects_do_not_move_under_a_handler_or_monitor() {
    for scope in [
        protected(),
        Stmt::TryFinally {
            body: vec![protected()],
            cleanup: vec![Stmt::Basic(B5)],
            handler: B10,
        },
        Stmt::Synchronized {
            monitor: V0,
            body: vec![protected()],
        },
    ] {
        check(vec![scope, Stmt::Basic(B6)], false);
    }
}

#[test]
fn cleanup_exit_keeps_its_override_of_pending_return_and_throw() {
    for pending in [Stmt::Return(B40), Stmt::Throw(B41)] {
        check(
            vec![Stmt::TryFinally {
                body: vec![Stmt::Basic(B1), pending],
                cleanup: vec![decision(0, vec![exit()]), Stmt::Basic(B5)],
                handler: B10,
            }],
            false,
        );
    }
}

fn switch(owner: Vec<Stmt>, other: Vec<Stmt>) -> Stmt {
    Stmt::Switch {
        region: SWITCH,
        head: B0,
        cases: vec![
            SwitchCase {
                keys: vec![0],
                default: false,
                body: owner,
            },
            SwitchCase {
                keys: vec![1],
                default: true,
                body: other,
            },
        ],
    }
}

#[test]
fn switch_exits_rebind_inside_handlers_and_cleanup_without_moving_effects() {
    for other in [
        protected(),
        Stmt::Synchronized {
            monitor: V0,
            body: vec![protected()],
        },
        Stmt::TryFinally {
            body: vec![protected()],
            cleanup: vec![Stmt::Basic(B5), decision(1, vec![exit()])],
            handler: B10,
        },
        Stmt::TryFinally {
            body: vec![decision(0, vec![Stmt::Return(B40)]), Stmt::Throw(B41)],
            cleanup: vec![Stmt::Basic(B5), exit()],
            handler: B10,
        },
    ] {
        check(
            vec![
                switch(
                    vec![Stmt::Basic(B1), Stmt::Break { target: SWITCH }],
                    vec![other],
                ),
                Stmt::Basic(B6),
            ],
            true,
        );
    }
}

#[test]
fn switch_owner_runs_its_suffix_after_handlers_and_monitor_release() {
    for owner in [
        Stmt::Try {
            body: vec![Stmt::Basic(B1), Stmt::Break { target: SWITCH }],
            catches: vec![catch(None, 0, vec![Stmt::Basic(B2), exit()])],
        },
        Stmt::Synchronized {
            monitor: V0,
            body: vec![Stmt::Basic(B1), Stmt::Break { target: SWITCH }],
        },
        Stmt::TryFinally {
            body: vec![Stmt::Basic(B1), Stmt::Break { target: SWITCH }],
            cleanup: vec![Stmt::Basic(B5), decision(1, vec![exit()])],
            handler: B10,
        },
    ] {
        check(
            vec![
                switch(vec![owner], vec![Stmt::Basic(B3), exit()]),
                Stmt::Basic(B6),
            ],
            true,
        );
    }
}

#[test]
fn empty_case_completion_keeps_a_shared_suffix_outside_protected_owners() {
    for owner in [
        Stmt::Try {
            body: vec![
                Stmt::Basic(B1),
                decision(0, vec![Stmt::Break { target: SWITCH }]),
            ],
            catches: vec![catch(None, 0, vec![Stmt::Basic(B2), exit()])],
        },
        Stmt::Synchronized {
            monitor: V0,
            body: vec![Stmt::Basic(B1), Stmt::Break { target: SWITCH }],
        },
        Stmt::TryFinally {
            body: vec![Stmt::Basic(B1), Stmt::Break { target: SWITCH }],
            cleanup: vec![Stmt::Basic(B5), decision(1, vec![exit()])],
            handler: B10,
        },
    ] {
        check(
            vec![
                Stmt::Switch {
                    region: SWITCH,
                    head: B0,
                    cases: vec![
                        SwitchCase {
                            keys: vec![0],
                            default: false,
                            body: vec![owner],
                        },
                        SwitchCase {
                            keys: vec![1],
                            default: false,
                            body: vec![Stmt::Break { target: SWITCH }],
                        },
                        SwitchCase {
                            keys: vec![2],
                            default: true,
                            body: vec![exit()],
                        },
                    ],
                },
                Stmt::Basic(B6),
            ],
            true,
        );
    }
}

#[test]
fn a_case_suffix_does_not_enter_the_owners_protected_scope() {
    for owner in [
        Stmt::Try {
            body: vec![
                Stmt::Basic(B1),
                decision(0, vec![Stmt::Break { target: SWITCH }]),
            ],
            catches: vec![catch(None, 0, vec![Stmt::Basic(B2), exit()])],
        },
        Stmt::Synchronized {
            monitor: V0,
            body: vec![
                Stmt::Basic(B1),
                decision(0, vec![Stmt::Break { target: SWITCH }]),
            ],
        },
    ] {
        check(
            vec![
                switch(
                    vec![owner, Stmt::Basic(B4), Stmt::Break { target: SWITCH }],
                    vec![Stmt::Basic(B3), exit()],
                ),
                Stmt::Basic(B6),
            ],
            false,
        );
    }
}

#[test]
fn switch_owner_cleanup_keeps_its_override_of_pending_exits() {
    for pending in [Stmt::Return(B40), Stmt::Throw(B41)] {
        check(
            vec![
                switch(
                    vec![Stmt::TryFinally {
                        body: vec![Stmt::Basic(B1), pending],
                        cleanup: vec![Stmt::Basic(B5), Stmt::Break { target: SWITCH }],
                        handler: B10,
                    }],
                    vec![Stmt::Basic(B3), exit()],
                ),
                Stmt::Basic(B6),
            ],
            false,
        );
    }
    for cleanup in [exit(), Stmt::Return(B40), Stmt::Throw(B41)] {
        check(
            vec![
                switch(
                    vec![Stmt::TryFinally {
                        body: vec![Stmt::Basic(B1), Stmt::Break { target: SWITCH }],
                        cleanup: vec![Stmt::Basic(B5), cleanup],
                        handler: B10,
                    }],
                    vec![Stmt::Basic(B3), exit()],
                ),
                Stmt::Basic(B6),
            ],
            false,
        );
    }
}
