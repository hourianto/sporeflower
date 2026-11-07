use super::*;
const B0: crate::index::BlockId = crate::index::BlockId::from_index(0);
const B1: crate::index::BlockId = crate::index::BlockId::from_index(1);
const B2: crate::index::BlockId = crate::index::BlockId::from_index(2);
const B3: crate::index::BlockId = crate::index::BlockId::from_index(3);
use crate::structure::{FlowBlock, FlowEnd};

fn graph(ends: Vec<FlowEnd>, protected: &[usize], handler: usize) -> FlowGraph {
    FlowGraph {
        entry: B0,
        blocks: ends
            .into_iter()
            .enumerate()
            .map(|(id, end)| FlowBlock {
                pc: id as u32,
                end,
                may_throw: protected.contains(&id),
                statements_empty: false,
                duplication_cost: None,
                is_loop_update: false,
                handlers: if protected.contains(&id) {
                    vec![HandlerEdge {
                        target: BlockId::from_index(handler),
                        catch_type: Some("java/io/IOException".into()),
                        table_index: 0,
                    }]
                } else {
                    vec![]
                },
            })
            .collect(),
    }
}

fn catch_count(body: &[Stmt]) -> usize {
    body.iter()
        .map(|stmt| match stmt {
            Stmt::Try { body, catches } => {
                catches.len()
                    + catch_count(body)
                    + catches
                        .iter()
                        .map(|catch| catch_count(&catch.body))
                        .sum::<usize>()
            }
            Stmt::If {
                then_body,
                else_body,
                ..
            } => catch_count(then_body) + catch_count(else_body),
            Stmt::Label { body, .. } | Stmt::Loop { body, .. } => catch_count(body),
            Stmt::Switch { cases, .. } => cases.iter().map(|case| catch_count(&case.body)).sum(),
            _ => 0,
        })
        .sum()
}

fn io_on_one(block: BlockId, body: Option<&[Stmt]>, handlers: &[HandlerEdge]) -> Vec<HandlerEdge> {
    let possible = body.map_or(block == B1, |body| {
        body.iter().any(|stmt| {
            matches!(
                stmt,
                Stmt::Basic(B1)
                    | Stmt::Statements {
                        block: B1,
                        start: 0,
                        end: 1
                    }
            )
        })
    });
    if possible {
        handlers.to_vec()
    } else {
        Vec::new()
    }
}

#[test]
fn a_whole_owned_body_keeps_coverage_over_its_non_io_operations() {
    let graph = graph(
        vec![FlowEnd::Goto(B1), FlowEnd::Return, FlowEnd::Return],
        &[0, 1],
        2,
    );
    let eligibility = Eligibility::new(&graph, &BTreeMap::new(), Some(&io_on_one));
    let tree = crate::structure::structure_graph_inner(&graph, &eligibility).unwrap();
    assert_eq!(catch_count(&tree.body), 1);
    let Stmt::Try { body, .. } = &tree.body[0] else {
        panic!("{tree:#?}");
    };
    assert!(body.iter().any(|stmt| matches!(stmt, Stmt::Basic(B0))));
    assert!(body.iter().any(|stmt| matches!(stmt, Stmt::Basic(B1))));
}

#[test]
fn disconnected_same_signature_regions_do_not_share_eligibility() {
    let graph = graph(
        vec![
            FlowEnd::Branch {
                then_target: B1,
                else_target: B2,
            },
            FlowEnd::Goto(B3),
            FlowEnd::Goto(B3),
            FlowEnd::Return,
            FlowEnd::Return,
        ],
        &[1, 2],
        4,
    );
    let eligibility = Eligibility::new(&graph, &BTreeMap::new(), Some(&io_on_one));
    let tree = crate::structure::structure_graph_inner(&graph, &eligibility).unwrap();
    assert_eq!(catch_count(&tree.body), 1, "{tree:#?}");
    assert_eq!(tree.statistics.dispatch_regions, 0);
}

#[test]
fn an_impossible_catch_does_not_remove_its_live_normal_entry() {
    let graph = graph(
        vec![
            FlowEnd::Branch {
                then_target: B1,
                else_target: B2,
            },
            FlowEnd::Throw,
            FlowEnd::Goto(B3),
            FlowEnd::Return,
        ],
        &[1],
        3,
    );
    let none = |_, _: Option<&[Stmt]>, _: &[HandlerEdge]| Vec::new();
    let eligibility = Eligibility::new(&graph, &BTreeMap::new(), Some(&none));
    let tree = crate::structure::structure_graph_inner(&graph, &eligibility).unwrap();
    assert_eq!(catch_count(&tree.body), 0);
    assert!(format!("{:?}", tree.body).contains("Return(BlockId(3))"));
}

#[test]
fn macro_refresh_uses_the_actual_cleanup_slice() {
    let graph = graph(
        vec![FlowEnd::Goto(B1), FlowEnd::Return, FlowEnd::Return],
        &[0, 1],
        2,
    );
    for end in [0, 1] {
        let effects = BTreeMap::from([(
            B1,
            vec![Stmt::Statements {
                block: B1,
                start: 0,
                end,
            }],
        )]);
        let mut eligibility = Eligibility::new(&graph, &effects, Some(&io_on_one));
        eligibility.install(
            B0,
            &crate::structure::installation::Body::Unexpanded(vec![Stmt::Basic(B1)]),
            &graph.blocks[B0].handlers,
            &[B0, B1],
        );
        assert_eq!(
            eligibility.caught(&[B0], &graph.blocks[B0].handlers).len(),
            end
        );
    }
}

#[test]
fn nonthrowing_connectors_cannot_authorize_shadowed_source_catches() {
    let mut graph = graph(
        vec![
            FlowEnd::Goto(B1),
            FlowEnd::Return,
            FlowEnd::Return,
            FlowEnd::Return,
        ],
        &[0],
        2,
    );
    let first = HandlerEdge {
        target: B2,
        catch_type: Some("java/lang/Exception".into()),
        table_index: 0,
    };
    let outer = HandlerEdge {
        target: B3,
        catch_type: Some("java/lang/SecurityException".into()),
        table_index: 1,
    };
    graph.blocks[B0].handlers = vec![first, outer.clone()];
    // The protected operation's empty continuation is outside the inner
    // Exception range. Its lexical outer coverage remains reconstruction
    // evidence, but cannot make SecurityException executable at that operation.
    graph.blocks[B1].handlers = vec![outer.clone()];
    let filter = |_, _: Option<&[Stmt]>, handlers: &[HandlerEdge]| {
        crate::cfg::dispatch::effective_with(handlers, |later, earlier| {
            later == "java/lang/SecurityException" && earlier == "java/lang/Exception"
        })
        .cloned()
        .collect()
    };
    let eligibility = Eligibility::new(&graph, &BTreeMap::new(), Some(&filter));
    let tree = crate::structure::structure_graph_inner(&graph, &eligibility).unwrap();
    assert_eq!(catch_count(&tree.body), 1, "{tree:#?}");
    assert_eq!(tree.statistics.dispatch_regions, 0);
    let catches = crate::structure::tree::walk(&tree.body)
        .filter_map(|statement| match statement {
            Stmt::Try { catches, .. } => Some(catches.as_slice()),
            _ => None,
        })
        .flatten()
        .map(|catch| catch.handler)
        .collect::<Vec<_>>();
    assert_eq!(catches, [B2]);

    // An earlier throwing operation covered only by the outer handler still
    // needs that catch. Removing eligibility from the connector must not
    // remove an executable outer scope along with the shadowed sibling.
    graph.entry = graph.blocks.push(FlowBlock {
        pc: 4,
        end: FlowEnd::Goto(B0),
        may_throw: true,
        statements_empty: false,
        duplication_cost: None,
        is_loop_update: false,
        handlers: vec![outer],
    });
    let eligibility = Eligibility::new(&graph, &BTreeMap::new(), Some(&filter));
    let tree = crate::structure::structure_graph_inner(&graph, &eligibility).unwrap();
    assert_eq!(catch_count(&tree.body), 2, "{tree:#?}");
    assert_eq!(tree.statistics.dispatch_regions, 0);
    for statement in crate::structure::tree::walk(&tree.body) {
        if let Stmt::Try { catches, .. } = statement {
            assert_eq!(catches.len(), 1, "{tree:#?}");
        }
    }
}
