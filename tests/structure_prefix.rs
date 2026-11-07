const B0: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(0);
const B1: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(1);
const B2: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(2);
const B3: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(3);
const B4: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(4);
use sporeflower::{
    cfg::HandlerEdge,
    structure::{self, FlowBlock, FlowEnd, FlowGraph, Stmt},
};

fn block(pc: u32, end: FlowEnd) -> FlowBlock {
    FlowBlock {
        pc,
        statements_empty: false,
        duplication_cost: None,
        is_loop_update: false,
        end,
        handlers: vec![],
        may_throw: true,
    }
}

#[test]
fn one_shot_initialization_is_outside_an_irreducible_dispatch_loop() {
    let graph = FlowGraph {
        entry: B0,
        blocks: vec![
            block(0, FlowEnd::Goto(B1)),
            block(
                1,
                FlowEnd::Branch {
                    then_target: B2,
                    else_target: B3,
                },
            ),
            block(
                2,
                FlowEnd::Branch {
                    then_target: B3,
                    else_target: B4,
                },
            ),
            block(
                3,
                FlowEnd::Branch {
                    then_target: B2,
                    else_target: B4,
                },
            ),
            block(4, FlowEnd::Return),
        ]
        .into(),
    };
    let tree = structure::structure_graph(&graph).unwrap();
    assert!(
        matches!(
            tree.body.as_slice(),
            [Stmt::Basic(B0), Stmt::Basic(B1), Stmt::Dispatch { .. }]
        ),
        "{:#?}",
        tree.body
    );
    let Stmt::Dispatch { entry, cases, .. } = &tree.body[2] else {
        unreachable!()
    };
    assert_eq!(*entry, B1);
    assert!(cases.iter().all(|case| case.head != B0));
    assert!(
        cases
            .iter()
            .find(|case| case.head == B1)
            .unwrap()
            .body
            .is_empty()
    );
    assert!(
        cases
            .iter()
            .find(|case| case.head == B2)
            .unwrap()
            .body
            .iter()
            .any(|statement| matches!(statement, Stmt::Basic(B2)))
    );
}

#[test]
fn a_reentered_entry_keeps_its_effects_inside_the_loop() {
    let graph = FlowGraph {
        entry: B0,
        blocks: vec![
            block(
                0,
                FlowEnd::Branch {
                    then_target: B1,
                    else_target: B2,
                },
            ),
            block(
                1,
                FlowEnd::Branch {
                    then_target: B0,
                    else_target: B2,
                },
            ),
            block(
                2,
                FlowEnd::Branch {
                    then_target: B1,
                    else_target: B3,
                },
            ),
            block(3, FlowEnd::Return),
        ]
        .into(),
    };
    let tree = structure::structure_graph(&graph).unwrap();
    assert!(
        !matches!(tree.body.first(), Some(Stmt::Basic(B0))),
        "{:#?}",
        tree.body
    );
}

#[test]
fn protected_entry_effects_keep_their_exception_dispatch() {
    let mut graph = FlowGraph {
        entry: B0,
        blocks: vec![
            block(
                0,
                FlowEnd::Branch {
                    then_target: B1,
                    else_target: B2,
                },
            ),
            block(
                1,
                FlowEnd::Branch {
                    then_target: B2,
                    else_target: B3,
                },
            ),
            block(
                2,
                FlowEnd::Branch {
                    then_target: B1,
                    else_target: B3,
                },
            ),
            block(3, FlowEnd::Return),
            block(4, FlowEnd::Throw),
        ]
        .into(),
    };
    graph.blocks[B0].handlers.push(HandlerEdge {
        target: B4,
        catch_type: None,
        table_index: 0,
    });
    let tree = structure::structure_graph(&graph).unwrap();
    assert!(
        !matches!(tree.body.first(), Some(Stmt::Basic(B0))),
        "protected effects moved outside catch: {:#?}",
        tree.body
    );
}
