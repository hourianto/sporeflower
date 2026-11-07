use super::*;
const B0: crate::index::BlockId = crate::index::BlockId::from_index(0);
const B1: crate::index::BlockId = crate::index::BlockId::from_index(1);
const B2: crate::index::BlockId = crate::index::BlockId::from_index(2);
const B3: crate::index::BlockId = crate::index::BlockId::from_index(3);
const B4: crate::index::BlockId = crate::index::BlockId::from_index(4);
const B5: crate::index::BlockId = crate::index::BlockId::from_index(5);
const B6: crate::index::BlockId = crate::index::BlockId::from_index(6);
const B7: crate::index::BlockId = crate::index::BlockId::from_index(7);
const B20: crate::index::BlockId = crate::index::BlockId::from_index(20);
const B21: crate::index::BlockId = crate::index::BlockId::from_index(21);
const B22: crate::index::BlockId = crate::index::BlockId::from_index(22);
use crate::{cfg::HandlerEdge, structure::FlowBlock};

fn corridor() -> FlowGraph {
    use FlowEnd::*;
    FlowGraph {
        entry: B0,
        blocks: [
            Goto(B1),
            Branch {
                then_target: B2,
                else_target: B5,
            },
            Branch {
                then_target: B3,
                else_target: B4,
            },
            Goto(B5),
            Goto(B1),
            Goto(B6),
            Return,
        ]
        .into_iter()
        .enumerate()
        .map(|(id, end)| FlowBlock {
            pc: id as u32,
            end,
            statements_empty: false,
            duplication_cost: None,
            is_loop_update: false,
            handlers: vec![],
            may_throw: false,
        })
        .collect(),
    }
}

#[test]
fn ordinary_and_lexical_loops_own_the_success_work_before_their_shared_follow() {
    let graph = corridor();
    let analysis = Analysis::new(&graph);
    let index = analysis.loop_at[B1].unwrap();
    assert_eq!(analysis.loops[index].exits, [B5]);
    assert!(analysis.loops[index].contains(B3));
    for tree in [
        recover(&graph, &analysis).unwrap(),
        crate::structure::stackify::stackify(&graph, &analysis).unwrap(),
    ] {
        let body = crate::structure::tree::walk(&tree)
            .find_map(|statement| match statement {
                Stmt::Loop {
                    header: B1, body, ..
                } => Some(body),
                _ => None,
            })
            .unwrap();
        assert!(
            crate::structure::tree::walk(body)
                .any(|statement| matches!(statement, Stmt::Basic(B3)))
        );
        assert!(
            !crate::structure::tree::walk(body)
                .any(|statement| matches!(statement, Stmt::Basic(B5)))
        );
    }
}

#[test]
fn a_following_search_does_not_detach_the_first_searchs_success_work() {
    const B8: BlockId = BlockId::from_index(8);
    const B9: BlockId = BlockId::from_index(9);
    const B10: BlockId = BlockId::from_index(10);
    use FlowEnd::*;
    let graph = FlowGraph {
        entry: B0,
        blocks: [
            Goto(B1),
            Branch {
                then_target: B2,
                else_target: B5,
            },
            Branch {
                then_target: B3,
                else_target: B4,
            },
            Goto(B10),
            Goto(B1),
            Goto(B6),
            Branch {
                then_target: B7,
                else_target: B10,
            },
            Branch {
                then_target: B8,
                else_target: B9,
            },
            Goto(B10),
            Goto(B6),
            Return,
        ]
        .into_iter()
        .enumerate()
        .map(|(id, end)| FlowBlock {
            pc: id as u32,
            end,
            statements_empty: false,
            duplication_cost: None,
            is_loop_update: id == 4 || id == 9,
            handlers: vec![],
            may_throw: false,
        })
        .collect(),
    };
    let analysis = Analysis::new(&graph);
    let first = &analysis.loops[analysis.loop_at[B1].unwrap()];
    assert!(first.contains(B3));
    assert!(!first.contains(B5));
    assert!(!first.contains(B6));
    assert_eq!(first.exits, [B5, B10]);
    let tree = crate::structure::stackify::stackify(&graph, &analysis).unwrap();
    let body = crate::structure::tree::walk(&tree)
        .find_map(|statement| match statement {
            Stmt::Loop {
                header: B1, body, ..
            } => Some(body),
            _ => None,
        })
        .unwrap();
    assert!(
        crate::structure::tree::walk(body).any(|statement| matches!(statement, Stmt::Basic(B3)))
    );
    assert!(
        !crate::structure::tree::walk(body)
            .any(|statement| matches!(statement, Stmt::Basic(B5) | Stmt::Basic(B10)))
    );
}

#[test]
fn inner_guard_completion_stays_outside_when_mismatch_continues_the_outer_loop() {
    const B8: BlockId = BlockId::from_index(8);
    use FlowEnd::*;
    let graph = FlowGraph {
        entry: B0,
        blocks: [
            Goto(B1),
            Branch {
                then_target: B2,
                else_target: B8,
            },
            Goto(B3),
            Branch {
                then_target: B4,
                else_target: B6,
            },
            Branch {
                then_target: B5,
                else_target: B7,
            },
            Goto(B3),
            Goto(B7),
            Goto(B1),
            Return,
        ]
        .into_iter()
        .enumerate()
        .map(|(id, end)| FlowBlock {
            pc: id as u32,
            end,
            statements_empty: false,
            duplication_cost: None,
            is_loop_update: id == 5 || id == 7,
            handlers: vec![],
            may_throw: false,
        })
        .collect(),
    };
    let analysis = Analysis::new(&graph);
    assert!(!analysis.loops[analysis.loop_at[B3].unwrap()].contains(B6));
    for tree in [
        recover(&graph, &analysis).unwrap(),
        crate::structure::stackify::stackify(&graph, &analysis).unwrap(),
    ] {
        let body = crate::structure::tree::walk(&tree)
            .find_map(|statement| match statement {
                Stmt::Loop {
                    header: B3, body, ..
                } => Some(body),
                _ => None,
            })
            .unwrap();
        assert!(
            !crate::structure::tree::walk(body)
                .any(|statement| matches!(statement, Stmt::Basic(B6)))
        );
    }
}

#[test]
fn exit_corridor_requires_exclusive_normal_and_exception_ownership() {
    fn follow(graph: &FlowGraph) -> RecoveryResult<(Option<BlockId>, Vec<BlockId>)> {
        let analysis = Analysis::new(graph);
        Builder::new(graph, &analysis).loop_follow(analysis.loop_at[B1].unwrap())
    }
    let graph = corridor();
    assert!(follow(&graph).is_ok());

    let mut nonthrowing_range = graph.clone();
    nonthrowing_range.blocks[B1].handlers.push(HandlerEdge {
        target: B6,
        catch_type: Some("java/lang/Exception".into()),
        table_index: 0,
    });
    assert!(follow(&nonthrowing_range).is_ok());

    let mut external = graph.clone();
    external.blocks[B0].end = FlowEnd::Branch {
        then_target: B1,
        else_target: B3,
    };
    assert!(follow(&external).is_err());

    let mut exceptional = graph.clone();
    exceptional.blocks[B0].handlers.push(HandlerEdge {
        target: B3,
        catch_type: Some("java/lang/Exception".into()),
        table_index: 0,
    });
    assert!(follow(&exceptional).is_err());

    let mut cyclic = graph;
    cyclic.blocks[B3].end = FlowEnd::Branch {
        then_target: B3,
        else_target: B5,
    };
    assert!(follow(&cyclic).is_err());
}

#[test]
fn optional_arm_follow_excludes_owned_abrupt_exits() {
    use FlowEnd::*;
    let mut graph = FlowGraph {
        entry: B0,
        blocks: [
            Goto(B1),
            Branch {
                then_target: B2,
                else_target: B7,
            },
            Branch {
                then_target: B3,
                else_target: B5,
            },
            Branch {
                then_target: B4,
                else_target: B6,
            },
            Goto(B5),
            Goto(B6),
            Goto(B1),
            Return,
        ]
        .into_iter()
        .enumerate()
        .map(|(id, end)| FlowBlock {
            pc: id as u32,
            end,
            statements_empty: false,
            duplication_cost: None,
            is_loop_update: id == 6,
            handlers: vec![],
            may_throw: false,
        })
        .collect(),
    };
    assert!(recover(&graph, &Analysis::new(&graph)).is_ok());

    // B4 also enters through an independent arm, so B3 cannot claim it while
    // treating B5 as its optional-arm follow.
    graph.blocks[B2].end = Branch {
        then_target: B3,
        else_target: B4,
    };
    graph.blocks[B3].end = Branch {
        then_target: B4,
        else_target: B5,
    };
    let analysis = Analysis::new(&graph);
    let mut builder = Builder::new(&graph, &analysis);
    builder.loops.push(LoopScope {
        index: analysis.loop_at[B1].unwrap(),
        region: 0,
        follow: Some(B7),
        terminal_nodes: vec![],
        continuation: Some(B6),
    });
    assert!(
        builder
            .branch_arms(
                B2,
                &Predicate::Atom {
                    head: B2,
                    negated: false
                },
                B3,
                B4,
                Some(B6),
                &[]
            )
            .is_err()
    );
    // The enclosing closed region can still retain B4 once in lexical form.
    assert!(recover(&graph, &analysis).is_ok());
}

fn graph(ends: Vec<FlowEnd>) -> FlowGraph {
    FlowGraph {
        entry: B0,
        blocks: ends
            .into_iter()
            .enumerate()
            .map(|(id, end)| FlowBlock {
                pc: id as u32,
                end,
                statements_empty: false,
                duplication_cost: None,
                is_loop_update: false,
                handlers: vec![],
                may_throw: false,
            })
            .collect(),
    }
}

#[test]
fn shared_tail_rejection_names_the_limit_without_consuming_ownership() {
    let mut graph = graph(vec![FlowEnd::Goto(B1), FlowEnd::Return]);
    graph.blocks[B0].duplication_cost = Some(1);
    graph.blocks[B1].duplication_cost = Some(49);
    let analysis = Analysis::new(&graph);
    let mut builder = Builder::new(&graph, &analysis);
    assert_eq!(
        builder.shared_tail(B0, None).unwrap_err(),
        RecoveryRejection::at(B1, RejectionReason::DuplicationLimit)
    );
    assert!(builder.visited.iter().all(|seen| !seen));
    assert_eq!(builder.duplicated_cost, 0);

    graph.blocks[B1].duplication_cost = None;
    let mut builder = Builder::new(&graph, &analysis);
    assert_eq!(
        builder.shared_tail(B0, None).unwrap_err(),
        RecoveryRejection::at(B1, RejectionReason::SharedTail)
    );
    assert!(builder.visited.iter().all(|seen| !seen));
}

#[test]
fn opaque_region_exits_bind_to_the_enclosing_loop() {
    use FlowEnd::*;
    let graph = graph(vec![
        Goto(B1),
        Branch {
            then_target: B2,
            else_target: B4,
        },
        Multi(vec![(B20, B1), (B21, B3), (B22, B4)]),
        Goto(B1),
        Return,
    ]);
    let body = recover(&graph, &Analysis::new(&graph)).unwrap();
    assert_eq!(crate::structure::statistics(&body).labeled_regions, 0);
    let Stmt::Loop { body, region, .. } = &body[1] else {
        panic!("{body:#?}")
    };
    let exits = body
        .iter()
        .find_map(|statement| match statement {
            Stmt::BoundRegion { exits, .. } => Some(exits),
            _ => None,
        })
        .unwrap();
    assert!(matches!(
        exits[0].1.as_slice(),
        [Stmt::Continue { header: B1 }]
    ));
    assert!(exits[1].1.is_empty());
    assert!(matches!(exits[2].1.as_slice(), [Stmt::Break { target }] if target == region));
}

#[test]
fn opaque_region_fallback_does_not_spill_the_enclosing_switch_cases() {
    use FlowEnd::*;
    let graph = graph(vec![
        Switch {
            cases: vec![(0, B1)],
            default: B5,
        },
        Multi(vec![(B20, B2), (B21, B3)]),
        Goto(B4),
        Goto(B4),
        Goto(B6),
        Goto(B6),
        Return,
    ]);
    let body = recover(&graph, &Analysis::new(&graph)).unwrap();
    let Stmt::Switch { cases, .. } = &body[1] else {
        panic!("{body:#?}")
    };
    assert!(
        cases[0]
            .body
            .iter()
            .any(|statement| matches!(statement, Stmt::Basic(B4)))
    );
    assert!(
        cases[1]
            .body
            .iter()
            .any(|statement| matches!(statement, Stmt::Basic(B5)))
    );
    assert!(matches!(
        body.as_slice(),
        [
            Stmt::Basic(B0),
            Stmt::Switch { .. },
            Stmt::Basic(B6),
            Stmt::Return(B6)
        ]
    ));
}
