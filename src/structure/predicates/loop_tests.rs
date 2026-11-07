use super::*;
const B0: crate::index::BlockId = crate::index::BlockId::from_index(0);
const B4: crate::index::BlockId = crate::index::BlockId::from_index(4);
const B8: crate::index::BlockId = crate::index::BlockId::from_index(8);
const B20: crate::index::BlockId = crate::index::BlockId::from_index(20);
const B100: crate::index::BlockId = crate::index::BlockId::from_index(100);
const B101: crate::index::BlockId = crate::index::BlockId::from_index(101);
const B102: crate::index::BlockId = crate::index::BlockId::from_index(102);
const B200: crate::index::BlockId = crate::index::BlockId::from_index(200);
const B300: crate::index::BlockId = crate::index::BlockId::from_index(300);
const B301: crate::index::BlockId = crate::index::BlockId::from_index(301);
const B302: crate::index::BlockId = crate::index::BlockId::from_index(302);
const B400: crate::index::BlockId = crate::index::BlockId::from_index(400);
const B500: crate::index::BlockId = crate::index::BlockId::from_index(500);
const B600: crate::index::BlockId = crate::index::BlockId::from_index(600);
const B700: crate::index::BlockId = crate::index::BlockId::from_index(700);
const B800: crate::index::BlockId = crate::index::BlockId::from_index(800);
const V10: crate::index::ValueId = crate::index::ValueId::from_index(10);
use crate::classfile::JvmType;
use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Eq)]
enum Exit {
    Break(usize),
    Continue(usize),
    Return(usize),
}

#[derive(Default)]
struct Machine {
    trace: Vec<(char, usize)>,
    tests: BTreeMap<usize, usize>,
    limit: usize,
}

impl Machine {
    fn predicate(&mut self, condition: &Predicate) -> bool {
        match condition {
            Predicate::Atom { head, negated } => {
                self.trace.push(('?', head.index()));
                let count = self.tests.entry(head.index()).or_default();
                let result = *count >= self.limit + head.index() % 2;
                *count += 1;
                result != *negated
            }
            Predicate::And(terms) => terms.iter().all(|term| self.predicate(term)),
            Predicate::Or(terms) => terms.iter().any(|term| self.predicate(term)),
            Predicate::Select { condition, yes, no } => {
                let selected = if self.predicate(condition) { yes } else { no };
                self.predicate(selected)
            }
        }
    }

    fn execute(&mut self, body: &[Stmt]) -> Option<Exit> {
        for statement in body {
            assert!(self.trace.len() < 1000, "unexpected nontermination");
            let exit = match statement {
                Stmt::Basic(block) => {
                    self.trace.push(('!', block.index()));
                    None
                }
                Stmt::Return(block) => {
                    self.trace.push(('R', block.index()));
                    Some(Exit::Return(block.index()))
                }
                Stmt::Break { target } => Some(Exit::Break(*target)),
                Stmt::Continue { header } => Some(Exit::Continue(header.index())),
                Stmt::If {
                    condition,
                    then_body,
                    else_body,
                } => {
                    let body = if self.predicate(condition) {
                        then_body
                    } else {
                        else_body
                    };
                    self.execute(body)
                }
                Stmt::Loop {
                    region,
                    header,
                    body,
                    continuation,
                } => loop {
                    match self.execute(body) {
                        None => {}
                        Some(Exit::Continue(target)) if target == header.index() => {}
                        Some(Exit::Break(target)) if target == *region => break None,
                        other => break other,
                    }
                    if let Some(block) = continuation {
                        self.trace.push(('U', block.index()));
                    }
                },
                Stmt::Label { region, body } => self
                    .execute(body)
                    .filter(|exit| *exit != Exit::Break(*region)),
                Stmt::TryFinally { body, cleanup, .. } => {
                    let pending = self.execute(body);
                    self.execute(cleanup).or(pending)
                }
                Stmt::Synchronized { monitor, body } => {
                    self.trace.push(('[', monitor.index()));
                    let pending = self.execute(body);
                    self.trace.push((']', monitor.index()));
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
}

fn guard(head: usize, transfer: Stmt) -> Stmt {
    Stmt::If {
        condition: Predicate::Atom {
            head: crate::index::BlockId::from_index(head),
            negated: false,
        },
        then_body: vec![transfer],
        else_body: vec![],
    }
}

fn fold(statement: Stmt) -> Vec<Stmt> {
    let ir = MethodIr {
        entry: B0,
        values: vec![].into(),
        blocks: vec![].into(),
        locals: vec![].into(),

        parameters: vec![],
        return_type: JvmType::Void,

        initialization_sites: vec![],
    };
    normalize_statement(&ir, statement)
}

fn equivalent(original: &[Stmt], recovered: &[Stmt]) {
    for limit in 0..5 {
        let mut before = Machine {
            limit,
            ..Default::default()
        };
        let mut after = Machine {
            limit,
            ..Default::default()
        };
        assert_eq!(
            before.execute(original),
            after.execute(recovered),
            "limit={limit}, {recovered:#?}"
        );
        assert_eq!(before.trace, after.trace, "limit={limit}, {recovered:#?}");
    }
}

fn simple_loop(transfer: Stmt) -> Stmt {
    Stmt::Loop {
        region: 3,
        header: B4,
        continuation: Some(B200),
        body: vec![guard(0, transfer), Stmt::Basic(B100)],
    }
}

#[test]
fn outward_guards_preserve_updates_and_return_evaluation_under_cleanup() {
    let original = simple_loop(Stmt::Return(B300));
    let recovered = fold(original.clone());
    assert!(matches!(
        recovered.as_slice(),
        [Stmt::Loop { .. }, Stmt::Return(B300)]
    ));
    let wrap = |body| {
        vec![Stmt::TryFinally {
            body: vec![Stmt::Synchronized { monitor: V10, body }],
            cleanup: vec![Stmt::Basic(B400)],
            handler: B20,
        }]
    };
    equivalent(&wrap(vec![original]), &wrap(recovered));
}

#[test]
fn outward_breaks_and_continues_retain_their_enclosing_owners() {
    let original = simple_loop(Stmt::Break { target: 9 });
    let recovered = fold(original.clone());
    let wrap = |mut body: Vec<Stmt>| {
        body.push(Stmt::Basic(B500));
        vec![Stmt::Label { region: 9, body }, Stmt::Basic(B600)]
    };
    equivalent(&wrap(vec![original]), &wrap(recovered));

    let original = simple_loop(Stmt::Continue { header: B8 });
    let recovered = fold(original.clone());
    let wrap = |body: Vec<Stmt>| {
        vec![Stmt::Loop {
            region: 9,
            header: B8,
            continuation: Some(B700),
            body: std::iter::once(guard(1, Stmt::Break { target: 9 }))
                .chain(body)
                .chain(std::iter::once(Stmt::Basic(B800)))
                .collect(),
        }]
    };
    equivalent(&wrap(vec![original]), &wrap(recovered));
}

#[test]
fn an_inner_shadowing_loop_does_not_supply_an_outer_exit() {
    let original = Stmt::Loop {
        region: 3,
        header: B4,
        continuation: Some(B200),
        body: vec![
            guard(0, Stmt::Return(B300)),
            Stmt::Loop {
                region: 3,
                header: B8,
                continuation: None,
                body: vec![guard(2, Stmt::Break { target: 3 }), Stmt::Basic(B100)],
            },
        ],
    };
    let recovered = fold(original.clone());
    assert!(matches!(
        recovered.as_slice(),
        [Stmt::Loop { .. }, Stmt::Return(B300)]
    ));
    equivalent(&[original], &recovered);
}

#[test]
fn other_owned_breaks_and_nested_handler_guards_prevent_exit_factoring() {
    for body in [
        vec![
            guard(0, Stmt::Return(B300)),
            guard(2, Stmt::Break { target: 3 }),
        ],
        vec![guard(0, Stmt::Continue { header: B4 }), Stmt::Basic(B100)],
        vec![guard(0, Stmt::Break { target: 3 }), Stmt::Basic(B100)],
        vec![Stmt::TryFinally {
            body: vec![guard(0, Stmt::Return(B300))],
            cleanup: vec![Stmt::Basic(B100)],
            handler: B20,
        }],
    ] {
        let original = Stmt::Loop {
            region: 3,
            header: B4,
            continuation: None,
            body,
        };
        let before = format!("{original:?}");
        let recovered = fold(original);
        assert_eq!(format!("{:?}", recovered[0]), before);
        assert_eq!(recovered.len(), 1);
    }
}

#[test]
fn empty_guard_loops_preserve_condition_effects_and_owned_updates() {
    for continuation in [None, Some(B200)] {
        let original = Stmt::Loop {
            region: 3,
            header: B4,
            continuation,
            body: vec![
                Stmt::If {
                    condition: Predicate::Atom {
                        head: B0,
                        negated: true,
                    },
                    then_body: vec![Stmt::Continue { header: B4 }],
                    else_body: vec![],
                },
                Stmt::Break { target: 3 },
            ],
        };
        let recovered = fold(original.clone());
        let Stmt::Loop { body, .. } = &recovered[0] else {
            panic!("{recovered:#?}")
        };
        assert!(
            matches!(body.as_slice(), [Stmt::If { then_body, .. }] if matches!(then_body.as_slice(), [Stmt::Break { target: 3 }]))
        );
        equivalent(&[original], &recovered);
    }
}

#[test]
fn terminal_loop_follows_expose_the_ordinary_guard_without_losing_early_returns() {
    let original = Stmt::Loop {
        region: 3,
        header: B4,
        continuation: Some(B200),
        body: vec![
            guard(0, Stmt::Break { target: 9 }),
            guard(1, Stmt::Break { target: 3 }),
            Stmt::Basic(B100),
        ],
    };
    let ir = MethodIr {
        entry: B0,
        values: vec![].into(),
        blocks: vec![].into(),
        locals: vec![].into(),

        parameters: vec![],
        return_type: JvmType::Void,

        initialization_sites: vec![],
    };
    let mut bound = original.clone();
    let Stmt::Loop { region, body, .. } = &mut bound else {
        unreachable!()
    };
    redirect_exits(&ir, body, *region, &ImmediateExit::Return(B300).into());
    let mut recovered = normalize_statement(&ir, bound);
    assert!(matches!(
        recovered.as_slice(),
        [Stmt::Loop { .. }, Stmt::Break { target: 9 }]
    ));
    recovered.push(Stmt::Return(B300));
    let wrap = |body| {
        vec![
            Stmt::Label { region: 9, body },
            Stmt::Basic(B500),
            Stmt::Return(B301),
        ]
    };
    equivalent(&wrap(vec![original, Stmt::Return(B300)]), &wrap(recovered));
}

#[test]
fn a_direct_abrupt_arm_keeps_nested_decisions_in_the_enclosing_sequence() {
    let original = vec![Stmt::If {
        condition: Predicate::Atom {
            head: B0,
            negated: false,
        },
        then_body: vec![
            guard(1, Stmt::Return(B300)),
            Stmt::Basic(B100),
            Stmt::Return(B301),
        ],
        else_body: vec![Stmt::Basic(B101), Stmt::Return(B302)],
    }];
    let mut recovered = Vec::new();
    append(&mut recovered, original.clone());
    assert!(
        matches!(
            recovered.as_slice(),
            [
                Stmt::If {
                    condition: Predicate::Atom {
                        head: B0,
                        negated: true
                    },
                    ..
                },
                Stmt::If { .. },
                Stmt::Basic(B100),
                Stmt::Return(B301)
            ]
        ),
        "{recovered:#?}"
    );
    equivalent(&original, &recovered);
}

#[test]
fn a_break_continuation_does_not_enter_a_shadowing_destination() {
    let mut body = vec![
        guard(0, Stmt::Break { target: 1 }),
        Stmt::Label {
            region: 2,
            body: vec![guard(1, Stmt::Break { target: 1 }), Stmt::Basic(B100)],
        },
        Stmt::Basic(B101),
    ];
    let wrap = |body| {
        vec![
            Stmt::Label {
                region: 2,
                body: vec![Stmt::Label { region: 1, body }, Stmt::Break { target: 2 }],
            },
            Stmt::Basic(B102),
            Stmt::Return(B300),
        ]
    };
    let original = wrap(body.clone());
    let ir = MethodIr {
        entry: B0,
        values: vec![].into(),
        blocks: vec![].into(),
        locals: vec![].into(),

        parameters: vec![],
        return_type: JvmType::Void,

        initialization_sites: vec![],
    };
    redirect_exits(&ir, &mut body, 1, &ImmediateExit::Break(2).into());
    assert!(matches!(&body[0], Stmt::If { then_body, .. }
        if matches!(then_body.as_slice(), [Stmt::Break { target: 2 }])));
    equivalent(&original, &wrap(body));
}
