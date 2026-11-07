use super::*;
use crate::structure::eligibility::Eligibility;

const B0: BlockId = BlockId::from_index(0);
const B1: BlockId = BlockId::from_index(1);
const B2: BlockId = BlockId::from_index(2);
const B3: BlockId = BlockId::from_index(3);
const B4: BlockId = BlockId::from_index(4);

fn graph() -> FlowGraph {
    FlowGraph {
        entry: B0,
        blocks: [
            FlowEnd::Goto(B1),
            FlowEnd::Goto(B2),
            FlowEnd::Goto(B3),
            FlowEnd::Return,
            FlowEnd::Return,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, end)| FlowBlock {
            pc: index as u32,
            end,
            statements_empty: true,
            duplication_cost: Some(1),
            is_loop_update: true,
            handlers: vec![handler()],
            may_throw: true,
        })
        .collect(),
    }
}

fn handler() -> HandlerEdge {
    HandlerEdge {
        target: B4,
        catch_type: Some("java/io/IOException".into()),
        table_index: 0,
    }
}

fn region(entry: BlockId, nodes: Vec<BlockId>, body: Body) -> Region {
    Region {
        entry,
        nodes,
        outer: Vec::new(),
        end: FlowEnd::Goto(B3),
        body,
        may_throw: false,
    }
}

#[test]
fn nested_installation_retires_consumed_nodes_and_expands_once() {
    let mut graph = graph();
    let mut replacements = BTreeMap::new();
    install(
        &mut graph,
        &mut replacements,
        region(
            B1,
            vec![B1, B2],
            Body::Composed(vec![Stmt::Basic(B1), Stmt::Basic(B2)]),
        ),
        None,
    );
    install(
        &mut graph,
        &mut replacements,
        region(
            B0,
            vec![B0, B1],
            Body::Unexpanded(vec![Stmt::Basic(B0), Stmt::Basic(B1)]),
        ),
        None,
    );
    assert_eq!(replacements.keys().copied().collect::<Vec<_>>(), [B0]);
    for node in [B1, B2] {
        let block = &graph.blocks[node];
        assert!(matches!(block.end, FlowEnd::Unreachable));
        assert!(block.handlers.is_empty());
        assert!(!block.may_throw);
        assert!(!block.statements_empty);
        assert_eq!(block.duplication_cost, None);
        assert!(!block.is_loop_update);
    }
    assert_eq!(graph.blocks[B0].end.successors(), [B3]);
    assert!(!graph.blocks[B0].may_throw);
    assert_eq!(graph.blocks[B0].duplication_cost, None);
    let body = expand(vec![Stmt::Basic(B0)], &replacements);
    assert!(matches!(
        body.as_slice(),
        [Stmt::Basic(B0), Stmt::Basic(B1), Stmt::Basic(B2)]
    ));
}

#[test]
fn composed_prefix_keeps_original_operations_when_replacing_its_own_entry() {
    let mut graph = graph();
    let mut replacements = BTreeMap::from([(B0, vec![Stmt::Basic(B0), Stmt::Basic(B1)])]);
    let filter = |_: BlockId, body: Option<&[Stmt]>, handlers: &[HandlerEdge]| {
        if body.is_some_and(|body| {
            matches!(
                body,
                [
                    Stmt::Basic(B0),
                    Stmt::Statements {
                        block: B1,
                        start: 1,
                        end: 2
                    }
                ]
            )
        }) {
            handlers.to_vec()
        } else {
            Vec::new()
        }
    };
    let mut eligibility = Eligibility::new(&graph, &replacements, Some(&filter));
    let mut replacement = region(
        B0,
        vec![B0],
        Body::Composed(vec![
            Stmt::Basic(B0),
            Stmt::Statements {
                block: B1,
                start: 1,
                end: 2,
            },
        ]),
    );
    replacement.outer = vec![handler()];
    install(
        &mut graph,
        &mut replacements,
        replacement,
        Some(&mut eligibility),
    );
    assert_eq!(eligibility.caught(&[B0], &[handler()]).len(), 1);
    let body = expand(vec![Stmt::Basic(B0)], &replacements);
    assert!(matches!(
        body.as_slice(),
        [
            Stmt::Basic(B0),
            Stmt::Statements {
                block: B1,
                start: 1,
                end: 2
            }
        ]
    ));
}

#[test]
fn installation_refreshes_effects_before_retiring_consumed_eligibility() {
    for composed in [false, true] {
        let mut graph = graph();
        let complete = vec![Stmt::Statements {
            block: B1,
            start: 0,
            end: 1,
        }];
        let effects = BTreeMap::from([(B1, complete.clone())]);
        let filter = |_: BlockId, body: Option<&[Stmt]>, handlers: &[HandlerEdge]| {
            if body.is_some_and(|body| {
                matches!(
                    body,
                    [Stmt::Statements {
                        block: B1,
                        start: 0,
                        end: 1
                    }]
                )
            }) {
                handlers.to_vec()
            } else {
                Vec::new()
            }
        };
        let mut eligibility = Eligibility::new(&graph, &effects, Some(&filter));
        // Source macro contents and effect bodies are separate overlays.
        let mut replacements = BTreeMap::from([(B1, vec![Stmt::Basic(B1)])]);
        let body = if composed {
            Body::Composed(complete)
        } else {
            Body::Unexpanded(vec![Stmt::Basic(B1)])
        };
        let mut replacement = region(B0, vec![B0, B1], body);
        replacement.outer = vec![handler()];
        replacement.may_throw = true;
        install(
            &mut graph,
            &mut replacements,
            replacement,
            Some(&mut eligibility),
        );
        assert_eq!(eligibility.caught(&[B0], &[handler()]).len(), 1);
        assert!(eligibility.caught(&[B1], &[handler()]).is_empty());
        assert_eq!(graph.blocks[B0].handlers[0].target, B4);
        assert!(graph.blocks[B0].may_throw);
        assert!(!replacements.contains_key(&B1));
    }
}

#[test]
fn logical_exit_identity_survives_destination_mapping_and_dispatch() {
    let mut graph = graph();
    graph.blocks[B4].handlers.clear();
    let mut replacements = BTreeMap::new();
    let mut replacement = region(B0, vec![B0, B1, B2], Body::Composed(vec![Stmt::Jump(B2)]));
    replacement.end = FlowEnd::Multi(vec![(B2, B3)]);
    replacement
        .end
        .map_destinations(|target| if target == B3 { B4 } else { target });
    install(&mut graph, &mut replacements, replacement, None);
    assert!(matches!(&graph.blocks[B0].end, FlowEnd::Multi(exits) if exits == &[(B2, B4)]));
    let bound = expand(
        vec![Stmt::BoundRegion {
            head: B0,
            exits: vec![(B2, vec![Stmt::Return(B4)])],
        }],
        &replacements,
    );
    assert!(matches!(bound.as_slice(), [Stmt::Return(B4)]));
    let body = expand(dispatch(&graph, &Eligibility::default()), &replacements);
    let [Stmt::Dispatch { cases, .. }] = body.as_slice() else {
        panic!("{body:?}")
    };
    assert_eq!(
        cases.iter().map(|case| case.head).collect::<Vec<_>>(),
        [B0, B4]
    );
    assert!(matches!(
        cases[0].body.as_slice(),
        [Stmt::DispatchContinue {
            region: 0,
            target: B4
        }]
    ));
}
