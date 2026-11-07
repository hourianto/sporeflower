use super::*;
const B0: crate::index::BlockId = crate::index::BlockId::from_index(0);
const B1: crate::index::BlockId = crate::index::BlockId::from_index(1);
const B2: crate::index::BlockId = crate::index::BlockId::from_index(2);
const B3: crate::index::BlockId = crate::index::BlockId::from_index(3);
const B5: crate::index::BlockId = crate::index::BlockId::from_index(5);
const B10: crate::index::BlockId = crate::index::BlockId::from_index(10);
const B40: crate::index::BlockId = crate::index::BlockId::from_index(40);
const B41: crate::index::BlockId = crate::index::BlockId::from_index(41);
const B42: crate::index::BlockId = crate::index::BlockId::from_index(42);
const B43: crate::index::BlockId = crate::index::BlockId::from_index(43);
const B80: crate::index::BlockId = crate::index::BlockId::from_index(80);
const B81: crate::index::BlockId = crate::index::BlockId::from_index(81);
const B90: crate::index::BlockId = crate::index::BlockId::from_index(90);
const B91: crate::index::BlockId = crate::index::BlockId::from_index(91);
const B92: crate::index::BlockId = crate::index::BlockId::from_index(92);
use crate::structure::Predicate;

const LABEL: usize = 1000;
const SWITCH: usize = 2000;

#[derive(Debug, PartialEq, Eq)]
enum Exit {
    Break(usize),
    Return(usize),
    Throw(usize),
}

fn test(predicate: &Predicate, bits: u32, trace: &mut Vec<usize>) -> bool {
    match predicate {
        Predicate::Atom { head, negated } => {
            trace.push(20_000 + head.index());
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

fn execute(body: &[Stmt], selector: i32, bits: u32, trace: &mut Vec<usize>) -> Option<Exit> {
    for statement in body {
        let exit = match statement {
            Stmt::Basic(block) => {
                trace.push(block.index());
                None
            }
            Stmt::Break { target } => Some(Exit::Break(*target)),
            Stmt::Return(block) => {
                trace.push(30_000 + block.index());
                Some(Exit::Return(block.index()))
            }
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
                selector,
                bits,
                trace,
            ),
            Stmt::Label { region, body } => {
                execute(body, selector, bits, trace).filter(|exit| *exit != Exit::Break(*region))
            }
            Stmt::Switch {
                region,
                head,
                cases,
            } => {
                trace.push(10_000 + head.index());
                let start = cases
                    .iter()
                    .position(|case| case.keys.contains(&selector))
                    .or_else(|| cases.iter().position(|case| case.default));
                let mut result = None;
                if let Some(start) = start {
                    for case in &cases[start..] {
                        result = execute(&case.body, selector, bits, trace);
                        if result.is_some() {
                            break;
                        }
                    }
                }
                result.filter(|exit| *exit != Exit::Break(*region))
            }
            _ => panic!("unexpected statement {statement:?}"),
        };
        if exit.is_some() {
            return exit;
        }
    }
    None
}

fn equivalent(original: &[Stmt], recovered: &[Stmt], cases: usize, seed: u32) {
    for selector in -1..cases as i32 + 2 {
        let mut before = vec![];
        let mut after = vec![];
        let expected = execute(original, selector, seed, &mut before);
        let actual = execute(recovered, selector, seed, &mut after);
        assert_eq!(
            actual, expected,
            "selector={selector}, seed={seed}, {recovered:#?}"
        );
        assert_eq!(
            after, before,
            "selector={selector}, seed={seed}, {recovered:#?}"
        );
    }
}

#[test]
fn conditional_case_entry_shares_one_suffix_with_empty_completion_cases() {
    for corridor_effect in [false, true] {
        let original = vec![Stmt::Label {
            region: LABEL,
            body: vec![
                Stmt::Switch {
                    region: SWITCH,
                    head: B0,
                    cases: vec![
                        SwitchCase {
                            keys: vec![0],
                            default: false,
                            body: vec![Stmt::If {
                                condition: Predicate::Atom {
                                    head: B1,
                                    negated: false,
                                },
                                then_body: vec![Stmt::Basic(B40), Stmt::Break { target: LABEL }],
                                else_body: vec![Stmt::Break { target: SWITCH }],
                            }],
                        },
                        SwitchCase {
                            keys: vec![1],
                            default: false,
                            body: if corridor_effect {
                                vec![Stmt::Basic(B41), Stmt::Break { target: SWITCH }]
                            } else {
                                vec![Stmt::Break { target: SWITCH }]
                            },
                        },
                        SwitchCase {
                            keys: vec![2],
                            default: true,
                            body: vec![Stmt::Break { target: LABEL }],
                        },
                    ],
                },
                Stmt::Basic(B80),
            ],
        }];
        let recovered = simplify(original.clone());
        assert_eq!(
            matches!(recovered.as_slice(), [Stmt::Label { .. }]),
            corridor_effect
        );
        assert_eq!(
            crate::structure::tree::walk(&recovered)
                .filter(|statement| matches!(statement, Stmt::Basic(B80)))
                .count(),
            1
        );
        for seed in 0..4 {
            equivalent(&original, &recovered, 3, seed);
        }
    }
}

#[test]
fn switch_continuations_preserve_randomized_effects_and_fallthrough() {
    let mut random = 0x1357_9bdfu32;
    for seed in 0..2048u32 {
        let mut next = || {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            random
        };
        let count = 2 + next() as usize % 7;
        let owner = next() as usize % count;
        let exit = (owner + 1 + next() as usize % (count - 1)) % count;
        let default = next() as usize % count;
        let cases = (0..count)
            .map(|index| {
                let body = if index == exit {
                    vec![
                        Stmt::Basic(BlockId::from_index(index + 30)),
                        Stmt::Break { target: LABEL },
                    ]
                } else if index == owner {
                    match next() % 4 {
                        0 if index + 1 == count => {
                            vec![Stmt::Basic(BlockId::from_index(index + 40))]
                        }
                        1 => vec![
                            Stmt::Basic(BlockId::from_index(index + 40)),
                            Stmt::If {
                                condition: Predicate::Atom {
                                    head: B1,
                                    negated: false,
                                },
                                then_body: vec![Stmt::Break { target: SWITCH }],
                                else_body: vec![],
                            },
                            Stmt::Basic(BlockId::from_index(index + 100)),
                            Stmt::Break { target: SWITCH },
                        ],
                        2 => vec![Stmt::If {
                            condition: Predicate::Atom {
                                head: B2,
                                negated: false,
                            },
                            then_body: vec![
                                Stmt::Basic(BlockId::from_index(index + 40)),
                                Stmt::Break { target: SWITCH },
                            ],
                            else_body: vec![
                                Stmt::Basic(BlockId::from_index(index + 30)),
                                Stmt::Break { target: LABEL },
                            ],
                        }],
                        _ => vec![
                            Stmt::Basic(BlockId::from_index(index + 40)),
                            Stmt::Break { target: SWITCH },
                        ],
                    }
                } else {
                    match next() % 5 {
                        0 if index + 1 < count => vec![],
                        1 if index + 1 < count => {
                            vec![Stmt::Basic(BlockId::from_index(index + 50))]
                        }
                        2 => vec![
                            Stmt::Basic(BlockId::from_index(index + 60)),
                            Stmt::Return(BlockId::from_index(index)),
                        ],
                        3 => vec![Stmt::Throw(BlockId::from_index(index))],
                        _ => vec![Stmt::Break { target: 3000 }],
                    }
                };
                SwitchCase {
                    keys: vec![index as i32],
                    default: index == default,
                    body,
                }
            })
            .collect();
        let mut body = vec![
            Stmt::Basic(B10),
            Stmt::Switch {
                region: SWITCH,
                head: B0,
                cases,
            },
            Stmt::Basic(B80),
        ];
        match next() % 4 {
            0 => body.push(Stmt::Return(B90)),
            1 => body.push(Stmt::Break { target: LABEL }),
            2 => body.push(Stmt::If {
                condition: Predicate::Atom {
                    head: B3,
                    negated: false,
                },
                then_body: vec![Stmt::Return(B91)],
                else_body: vec![Stmt::Basic(B92)],
            }),
            _ => {}
        }
        let original = vec![Stmt::Label {
            region: LABEL,
            body,
        }];
        let recovered = simplify(original.clone());
        assert!(
            !matches!(recovered.first(), Some(Stmt::Label { .. })),
            "{recovered:#?}"
        );
        assert!(!crate::structure::contains_break(&recovered, LABEL));
        assert_eq!(
            crate::structure::tree::walk(&recovered)
                .filter(|statement| matches!(statement, Stmt::Basic(B80)))
                .count(),
            1,
            "the suffix must have one lexical owner"
        );
        equivalent(&original, &recovered, count, seed);
    }
}

#[test]
fn multiple_owners_or_a_partially_falling_through_owner_keep_their_label() {
    let case = |key, body| SwitchCase {
        keys: vec![key],
        default: key == 2,
        body,
    };
    let exit = || case(2, vec![Stmt::Break { target: LABEL }]);
    let owner = || case(0, vec![Stmt::Break { target: SWITCH }]);
    let candidates = vec![
        vec![
            owner(),
            case(1, vec![Stmt::Basic(B40), Stmt::Break { target: SWITCH }]),
            exit(),
        ],
        vec![owner(), case(1, vec![Stmt::Break { target: LABEL }])],
        vec![
            case(
                0,
                vec![Stmt::If {
                    condition: Predicate::Atom {
                        head: B1,
                        negated: false,
                    },
                    then_body: vec![Stmt::Break { target: SWITCH }],
                    else_body: vec![Stmt::Basic(B5)],
                }],
            ),
            exit(),
        ],
    ];
    for cases in candidates {
        let recovered = simplify_label(
            LABEL,
            vec![
                Stmt::Switch {
                    region: SWITCH,
                    head: B0,
                    cases,
                },
                Stmt::Basic(B80),
            ],
        );
        assert!(
            matches!(recovered.as_slice(), [Stmt::Label { region: LABEL, .. }]),
            "{recovered:#?}"
        );
    }
}

#[test]
fn a_switch_without_a_tail_can_rebind_its_exits_without_a_default() {
    let original = vec![Stmt::Label {
        region: LABEL,
        body: vec![Stmt::Switch {
            region: SWITCH,
            head: B0,
            cases: vec![SwitchCase {
                keys: vec![1, 2],
                default: false,
                body: vec![Stmt::Break { target: LABEL }],
            }],
        }],
    }];
    let recovered = simplify(original.clone());
    assert!(matches!(recovered.as_slice(), [Stmt::Switch { .. }]));
    equivalent(&original, &recovered, 3, 0);
}

#[test]
fn moving_a_tail_does_not_capture_an_outer_region_with_the_same_id() {
    for inherited in [false, true] {
        let switch = Stmt::Switch {
            region: SWITCH,
            head: B0,
            cases: vec![
                SwitchCase {
                    keys: vec![0],
                    default: false,
                    body: vec![Stmt::Break { target: SWITCH }],
                },
                SwitchCase {
                    keys: vec![1],
                    default: true,
                    body: vec![Stmt::Break { target: LABEL }],
                },
            ],
        };
        let prefix = if inherited {
            Stmt::If {
                condition: Predicate::Atom {
                    head: B1,
                    negated: false,
                },
                then_body: vec![switch],
                else_body: vec![Stmt::Break { target: LABEL }],
            }
        } else {
            switch
        };
        let original = vec![Stmt::Label {
            region: SWITCH,
            body: vec![
                Stmt::Label {
                    region: LABEL,
                    body: vec![prefix, Stmt::Basic(B80), Stmt::Break { target: SWITCH }],
                },
                // Distinguish the two continuations: the outer break must
                // still skip this effect after the inner label completes.
                Stmt::Basic(B90),
            ],
        }];
        let recovered = simplify(original.clone());
        for seed in 0..4 {
            equivalent(&original, &recovered, 2, seed);
        }
        assert!(
            matches!(recovered.as_slice(), [Stmt::Label { .. }]),
            "{recovered:#?}"
        );
    }
}

#[test]
fn a_tail_with_its_own_shadowing_switch_keeps_its_private_exits() {
    let original = vec![Stmt::Label {
        region: LABEL,
        body: vec![
            Stmt::Switch {
                region: SWITCH,
                head: B0,
                cases: vec![
                    SwitchCase {
                        keys: vec![0],
                        default: false,
                        body: vec![Stmt::Break { target: SWITCH }],
                    },
                    SwitchCase {
                        keys: vec![1],
                        default: true,
                        body: vec![Stmt::Break { target: LABEL }],
                    },
                ],
            },
            Stmt::Switch {
                region: SWITCH,
                head: B1,
                cases: vec![
                    SwitchCase {
                        keys: vec![0],
                        default: false,
                        body: vec![Stmt::Basic(B80), Stmt::Break { target: SWITCH }],
                    },
                    SwitchCase {
                        keys: vec![1],
                        default: true,
                        body: vec![Stmt::Return(B81)],
                    },
                ],
            },
        ],
    }];
    let recovered = simplify(original.clone());
    assert!(
        matches!(recovered.as_slice(), [Stmt::Switch { .. }]),
        "{recovered:#?}"
    );
    equivalent(&original, &recovered, 2, 0);
}

#[test]
fn case_exits_keep_lexical_shadowing_and_nested_switch_ownership() {
    let original = vec![Stmt::Label {
        region: LABEL,
        body: vec![
            Stmt::Switch {
                region: SWITCH,
                head: B0,
                cases: vec![
                    SwitchCase {
                        keys: vec![0],
                        default: false,
                        body: vec![Stmt::Switch {
                            region: SWITCH + 1,
                            head: B1,
                            cases: vec![
                                SwitchCase {
                                    keys: vec![0],
                                    default: false,
                                    body: vec![Stmt::Basic(B40), Stmt::Break { target: SWITCH }],
                                },
                                SwitchCase {
                                    keys: vec![1],
                                    default: true,
                                    body: vec![Stmt::Basic(B41), Stmt::Break { target: LABEL }],
                                },
                            ],
                        }],
                    },
                    SwitchCase {
                        keys: vec![1],
                        default: true,
                        body: vec![
                            Stmt::Label {
                                region: LABEL,
                                body: vec![Stmt::Basic(B42), Stmt::Break { target: LABEL }],
                            },
                            Stmt::Basic(B43),
                            Stmt::Break { target: LABEL },
                        ],
                    },
                ],
            },
            Stmt::Basic(B80),
        ],
    }];
    // Keep the nested binder in place to exercise the planner's lexical lookup.
    let Stmt::Label { body, .. } = original[0].clone() else {
        unreachable!()
    };
    let recovered = simplify_label(LABEL, body);
    assert!(matches!(recovered.as_slice(), [Stmt::Switch { .. }]));
    equivalent(&original, &recovered, 2, 0);
}

#[test]
fn case_rebinding_does_not_capture_a_shadowing_switch_or_label() {
    for nested in [
        Stmt::Label {
            region: SWITCH,
            body: vec![Stmt::Basic(B40), Stmt::Break { target: LABEL }],
        },
        Stmt::Switch {
            region: SWITCH,
            head: B1,
            cases: vec![SwitchCase {
                keys: vec![0],
                default: true,
                body: vec![Stmt::Basic(B40), Stmt::Break { target: LABEL }],
            }],
        },
    ] {
        let body = vec![
            Stmt::Switch {
                region: SWITCH,
                head: B0,
                cases: vec![
                    SwitchCase {
                        keys: vec![0],
                        default: false,
                        body: vec![Stmt::Basic(B41), Stmt::Break { target: SWITCH }],
                    },
                    SwitchCase {
                        keys: vec![1],
                        default: true,
                        body: vec![nested, Stmt::Basic(B42), Stmt::Return(B43)],
                    },
                ],
            },
            Stmt::Basic(B80),
        ];
        let original = vec![Stmt::Label {
            region: LABEL,
            body: body.clone(),
        }];
        let recovered = simplify_label(LABEL, body);
        assert!(matches!(recovered.as_slice(), [Stmt::Label { .. }]));
        equivalent(&original, &recovered, 2, 0);
    }
}
