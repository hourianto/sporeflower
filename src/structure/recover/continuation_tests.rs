use super::*;
const B0: crate::index::BlockId = crate::index::BlockId::from_index(0);
const B1: crate::index::BlockId = crate::index::BlockId::from_index(1);
const B2: crate::index::BlockId = crate::index::BlockId::from_index(2);
const B3: crate::index::BlockId = crate::index::BlockId::from_index(3);
const B4: crate::index::BlockId = crate::index::BlockId::from_index(4);
const B5: crate::index::BlockId = crate::index::BlockId::from_index(5);
const B6: crate::index::BlockId = crate::index::BlockId::from_index(6);
const B7: crate::index::BlockId = crate::index::BlockId::from_index(7);
const B8: crate::index::BlockId = crate::index::BlockId::from_index(8);
const B9: crate::index::BlockId = crate::index::BlockId::from_index(9);
const B10: crate::index::BlockId = crate::index::BlockId::from_index(10);
const B11: crate::index::BlockId = crate::index::BlockId::from_index(11);
const B12: crate::index::BlockId = crate::index::BlockId::from_index(12);
use crate::structure::{FlowBlock, FlowGraph, analysis::Analysis, recover::SwitchScope};

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
fn returning_fallthrough_groups_converge_before_the_conditional_suffix() {
    use FlowEnd::*;
    let input = graph(vec![
        Switch {
            cases: vec![(0, B1), (1, B2), (2, B3), (3, B4)],
            default: B5,
        },
        Goto(B2),
        Branch {
            then_target: B6,
            else_target: B7,
        },
        Goto(B4),
        Goto(B8),
        Goto(B8),
        Return,
        Goto(B8),
        Branch {
            then_target: B9,
            else_target: B10,
        },
        Goto(B11),
        Goto(B11),
        Return,
    ]);
    let analysis = Analysis::new(&input);
    let builder = Builder::new(&input, &analysis);
    assert_eq!(builder.join(B0), None);
    assert_eq!(
        builder.shared_follow(
            &[B0],
            &[
                B1,
                B2,
                BlockId::from_index(3),
                BlockId::from_index(4),
                BlockId::from_index(5)
            ],
            None
        ),
        Some(BlockId::from_index(8))
    );

    // The switch cannot absorb a branch body that also enters independently.
    let mut external = input;
    external.blocks.push(FlowBlock {
        pc: 12,
        end: Branch {
            then_target: B0,
            else_target: B7,
        },
        statements_empty: false,
        duplication_cost: None,
        is_loop_update: false,
        handlers: vec![],
        may_throw: false,
    });
    external.entry = B12;
    let analysis = Analysis::new(&external);
    assert_eq!(
        Builder::new(&external, &analysis).shared_follow(
            &[B0],
            &[
                B1,
                B2,
                BlockId::from_index(3),
                BlockId::from_index(4),
                BlockId::from_index(5)
            ],
            None
        ),
        None
    );
}

#[test]
fn explicit_stop_remains_normal_when_an_enclosing_switch_also_owns_it() {
    use FlowEnd::*;
    let input = graph(vec![
        Switch {
            cases: vec![(0, B1)],
            default: B6,
        },
        Switch {
            cases: vec![(0, B2), (1, B3)],
            default: B4,
        },
        Goto(B6),
        Goto(B5),
        Goto(B5),
        Goto(B6),
        Return,
    ]);
    let analysis = Analysis::new(&input);
    let mut builder = Builder::new(&input, &analysis);
    builder.switches.push(SwitchScope {
        region: 0,
        follow: Some(B6),
    });
    assert_eq!(
        builder.shared_follow(&[B1], &[B2, B3, BlockId::from_index(4)], Some(B6)),
        Some(BlockId::from_index(6))
    );
    // Without an explicit stop, an outer break remains an abrupt transfer.
    assert_eq!(
        builder.shared_follow(&[B1], &[B2, B3, BlockId::from_index(4)], None),
        Some(BlockId::from_index(5))
    );

    // A branch adds no break scope of its own: its returning arm must retain
    // the local join instead of swallowing it into the outer switch follow.
    let mut branch = input;
    branch.blocks[B1].end = Branch {
        then_target: B3,
        else_target: B4,
    };
    branch.blocks[B3].end = Branch {
        then_target: B5,
        else_target: B6,
    };
    let analysis = Analysis::new(&branch);
    let mut builder = Builder::new(&branch, &analysis);
    builder.switches.push(SwitchScope {
        region: 0,
        follow: Some(B6),
    });
    assert_eq!(
        builder.shared_follow(&[B1], &[B3, B4], Some(B6)),
        Some(BlockId::from_index(5))
    );
}

#[test]
fn a_last_frontier_must_be_reached_from_all_frontiers_without_cycling_back() {
    use FlowEnd::*;
    for (ends, frontier, stop, expected) in [
        (
            vec![
                Branch {
                    then_target: B1,
                    else_target: B2,
                },
                Return,
                Return,
            ],
            vec![1, 2],
            None,
            None,
        ),
        (
            vec![
                Goto(B1),
                Branch {
                    then_target: B0,
                    else_target: B2,
                },
                Return,
            ],
            vec![0, 1],
            None,
            None,
        ),
        (vec![Goto(B1), Goto(B2), Return], vec![0, 2], Some(1), None),
        (
            vec![
                Branch {
                    then_target: B1,
                    else_target: B2,
                },
                Return,
                Return,
            ],
            vec![0, 2],
            None,
            Some(2),
        ),
    ] {
        let frontier: Vec<_> = frontier.into_iter().map(BlockId::from_index).collect();
        let stop = stop.map(BlockId::from_index);
        let expected = expected.map(BlockId::from_index);
        let input = graph(ends);
        let analysis = Analysis::new(&input);
        let builder = Builder::new(&input, &analysis);
        assert_eq!(
            builder.last_shared_frontier(
                &vec![usize::MAX; input.blocks.len()].into(),
                &frontier,
                stop
            ),
            expected
        );
    }
}
