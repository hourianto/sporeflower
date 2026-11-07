const B0: crate::index::BlockId = crate::index::BlockId::from_index(0);
const B1: crate::index::BlockId = crate::index::BlockId::from_index(1);
const B2: crate::index::BlockId = crate::index::BlockId::from_index(2);
const B3: crate::index::BlockId = crate::index::BlockId::from_index(3);
const B4: crate::index::BlockId = crate::index::BlockId::from_index(4);
const B5: crate::index::BlockId = crate::index::BlockId::from_index(5);
const B8: crate::index::BlockId = crate::index::BlockId::from_index(8);
const B9: crate::index::BlockId = crate::index::BlockId::from_index(9);
const B11: crate::index::BlockId = crate::index::BlockId::from_index(11);
const B12: crate::index::BlockId = crate::index::BlockId::from_index(12);
const B13: crate::index::BlockId = crate::index::BlockId::from_index(13);
const B14: crate::index::BlockId = crate::index::BlockId::from_index(14);
const B99: crate::index::BlockId = crate::index::BlockId::from_index(99);
const V0: crate::index::ValueId = crate::index::ValueId::from_index(0);
use super::*;
use crate::index::BlockId;
use crate::structure::{Catch, DispatchCase, FlowEnd, Predicate, SwitchCase};

fn dispatch(region: usize, body: Vec<Stmt>, exits: Vec<(BlockId, usize)>) -> Stmt {
    Stmt::Dispatch {
        region,
        entry: B0,
        cases: vec![DispatchCase {
            head: B0,
            body,
            end: FlowEnd::Return,
            handlers: Vec::new(),
        }],
        exits,
    }
}

fn bodies_fixture() -> Vec<Stmt> {
    vec![
        Stmt::If {
            condition: Predicate::Atom {
                head: B0,
                negated: false,
            },
            then_body: vec![Stmt::Basic(B1)],
            else_body: vec![Stmt::Basic(B2)],
        },
        Stmt::Loop {
            region: 0,
            header: B0,
            body: vec![Stmt::Basic(B3)],
            continuation: Some(B99),
        },
        Stmt::Label {
            region: 1,
            body: vec![Stmt::Basic(B4)],
        },
        Stmt::Synchronized {
            monitor: V0,
            body: vec![Stmt::Basic(B5)],
        },
        Stmt::Switch {
            region: 2,
            head: B0,
            cases: [6, 7]
                .into_iter()
                .map(|block| SwitchCase {
                    keys: vec![block as i32],
                    default: false,
                    body: vec![Stmt::Basic(BlockId::from_index(block))],
                })
                .collect(),
        },
        Stmt::Try {
            body: vec![Stmt::Basic(B8)],
            catches: [9, 10]
                .into_iter()
                .map(|block| Catch {
                    catch_type: None,
                    handler: B0,
                    body: vec![Stmt::Basic(BlockId::from_index(block))],
                })
                .collect(),
        },
        Stmt::TryFinally {
            body: vec![Stmt::Basic(B11)],
            cleanup: vec![Stmt::Basic(B12)],
            handler: B0,
        },
        dispatch(3, vec![Stmt::Basic(B13)], Vec::new()),
        Stmt::BoundRegion {
            head: B0,
            exits: vec![(B0, vec![Stmt::Basic(B14)])],
        },
    ]
}

fn basic_blocks(body: &[Stmt]) -> Vec<usize> {
    walk(body)
        .filter_map(|statement| match statement {
            Stmt::Basic(block) => Some(block.index()),
            _ => None,
        })
        .collect()
}

#[test]
fn traversal_and_rewrites_cover_every_body_in_source_order() {
    let mut body = bodies_fixture();
    assert_eq!(basic_blocks(&body), (1..=14).collect::<Vec<_>>());
    // Changing each direct child catches dropped arms, catch bodies, dispatch
    // cases and opaque exit bindings without relying on the read-only walker.
    for statement in &mut body {
        for body in statement.bodies_mut() {
            let Stmt::Basic(block) = &mut body[0] else {
                panic!("fixture changed")
            };
            *block = BlockId::from_index(block.index() + 100);
        }
    }
    assert_eq!(basic_blocks(&body), (101..=114).collect::<Vec<_>>());
    let body: Vec<_> = body
        .into_iter()
        .map(|statement| {
            statement.map_bodies(|mut body| {
                let Stmt::Basic(block) = &mut body[0] else {
                    panic!("fixture changed")
                };
                *block = BlockId::from_index(block.index() - 100);
                body
            })
        })
        .collect();
    assert_eq!(basic_blocks(&body), (1..=14).collect::<Vec<_>>());
}

#[test]
fn renumbering_respects_shadowing_and_outward_dispatch_bindings() {
    let mut body = vec![Stmt::Label {
        region: 7,
        body: vec![
            Stmt::Loop {
                region: 7,
                header: B9,
                continuation: None,
                body: vec![Stmt::Break { target: 7 }],
            },
            Stmt::BoundRegion {
                head: B0,
                exits: vec![(
                    B0,
                    vec![Stmt::Label {
                        region: 7,
                        body: vec![Stmt::Break { target: 7 }],
                    }],
                )],
            },
            dispatch(
                7,
                vec![Stmt::DispatchContinue {
                    region: 7,
                    target: B0,
                }],
                vec![(BlockId::from_index(10), 7)],
            ),
            Stmt::Break { target: 7 },
        ],
    }];
    assert_eq!(crate::structure::renumber_regions_from(&mut body, 20), 24);
    let binders: Vec<_> = walk(&body)
        .filter_map(Stmt::binder)
        .map(Binder::region)
        .collect();
    assert_eq!(binders, vec![20, 21, 22, 23]);
    let targets: Vec<_> = walk(&body)
        .filter_map(|statement| match statement {
            Stmt::Break { target } | Stmt::DispatchContinue { region: target, .. } => Some(*target),
            _ => None,
        })
        .collect();
    assert_eq!(targets, vec![21, 22, 23, 20]);
    let Stmt::Dispatch { exits, .. } = walk(&body)
        .find(|s| matches!(s, Stmt::Dispatch { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(exits, &[(BlockId::from_index(10), 20)]);
}

#[test]
fn scoped_walk_restores_reused_loop_and_region_identities() {
    let body = vec![Stmt::Loop {
        region: 4,
        header: B9,
        continuation: None,
        body: vec![
            Stmt::Loop {
                region: 4,
                header: B9,
                continuation: None,
                body: vec![Stmt::Basic(B1)],
            },
            dispatch(4, vec![Stmt::Basic(B2)], Vec::new()),
            Stmt::BoundRegion {
                head: B0,
                exits: vec![(B0, vec![Stmt::Basic(B3)])],
            },
        ],
    }];
    let mut scopes = Vec::new();
    walk_scoped(&body, &mut |statement, active| {
        if let Stmt::Basic(block) = statement {
            scopes.push((*block, active.to_vec()));
        }
    });
    let outer = Binder::Loop {
        region: 4,
        header: B9,
    };
    assert_eq!(
        scopes,
        vec![
            (B1, vec![outer, outer]),
            (B2, vec![outer, Binder::Dispatch(4)]),
            (B3, vec![outer]),
        ]
    );
}

#[test]
fn completion_does_not_confuse_shadowed_breaks_with_dispatch_exits() {
    let nested = vec![Stmt::Label {
        region: 7,
        body: vec![Stmt::Break { target: 7 }],
    }];
    assert!(!crate::structure::contains_break(&nested, 7));
    assert!(crate::structure::contains_break(
        &[dispatch(7, Vec::new(), vec![(B0, 7)])],
        7
    ));
    assert!(crate::structure::contains_break(
        &[Stmt::BoundRegion {
            head: B0,
            exits: vec![(B0, vec![Stmt::Break { target: 7 }])],
        }],
        7
    ));
}

#[test]
fn binding_rewrites_outward_jumps_inside_opaque_exit_bodies() {
    let body = crate::structure::bindings::bind(
        vec![Stmt::BoundRegion {
            head: B0,
            exits: vec![(B1, vec![Stmt::Jump(B2)])],
        }],
        &[(B2, vec![Stmt::Return(B3)])],
    );
    assert!(!walk(&body).any(|statement| matches!(statement, Stmt::Jump(_))));
    assert!(walk(&body).any(|statement| matches!(statement, Stmt::Return(B3))));
}

#[test]
fn statistics_and_lowering_validation_reach_temporary_region_children() {
    let body = vec![Stmt::BoundRegion {
        head: B0,
        exits: vec![(
            B0,
            vec![dispatch(
                1,
                vec![Stmt::Label {
                    region: 2,
                    body: Vec::new(),
                }],
                Vec::new(),
            )],
        )],
    }];
    let statistics = crate::structure::statistics(&body);
    assert_eq!(statistics.labeled_regions, 1);
    assert_eq!(statistics.dispatch_regions, 1);
    assert_eq!(statistics.dispatch_blocks, 1);
    assert_eq!(crate::structure::count_labels(&body), 1);
    assert!(crate::structure::ensure_lowered(&body).is_err());
    assert!(
        crate::structure::ensure_lowered(&[dispatch(1, vec![Stmt::Jump(B0)], Vec::new())]).is_err()
    );
    assert!(
        crate::structure::ensure_lowered(&[dispatch(1, vec![Stmt::Return(B0)], Vec::new())])
            .is_ok()
    );
}
