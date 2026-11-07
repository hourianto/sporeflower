use super::*;
use crate::cfg::HandlerEdge;
use crate::index::test_ids::{B0, B1, B2, L0, L1};
use crate::ir::{IrBlock, Terminator};

fn method(block_count: usize) -> MethodIr {
    MethodIr {
        entry: B0,
        blocks: (0..block_count)
            .map(|_| IrBlock {
                caught_exception: None,
                pc: 0,
                statements: Vec::new(),
                terminator: Terminator::Return(None),
                handlers: Vec::new(),
            })
            .collect(),
        values: IdVec::new(),
        locals: IdVec::new(),
        parameters: Vec::new(),
        return_type: JvmType::Void,
        initialization_sites: Vec::new(),
    }
}

fn graph(operations: impl IntoIterator<Item = Op>) -> Graph {
    let mut graph = Graph {
        nodes: Vec::new(),
        sinks: Vec::new(),
        parameters: BTreeMap::new(),
    };
    for op in operations {
        graph.add(
            None,
            true,
            Site {
                block: B0,
                statement: 0,
            },
            JvmType::Int,
            op,
        );
    }
    graph
}

fn read() -> Op {
    Op::Read {
        local: L0,
        sources: BTreeSet::new(),
    }
}

fn guard(node: NodeId) -> Guard {
    Guard {
        key: Key {
            root: KeyRoot::Value(node),
            mask: None,
        },
        relation: relation(CompareOp::Ne),
        number: 42,
        dependencies: BTreeSet::new(),
    }
}

#[test]
fn joins_report_growth_and_stabilize_without_losing_later_locals() {
    let mut state = State::Definitions(vec![BTreeSet::from([0]), BTreeSet::from([1])].into());
    let other = State::Definitions(vec![BTreeSet::from([2]), BTreeSet::from([3])].into());
    assert!(state.join(&other));
    let State::Definitions(definitions) = &state else {
        panic!()
    };
    assert_eq!(definitions[L0], BTreeSet::from([0, 2]));
    assert_eq!(definitions[L1], BTreeSet::from([1, 3]));
    assert!(!state.join(&other));
    assert!(!state.join(&state.clone()));
}

#[test]
fn guard_joins_keep_only_common_facts_and_stabilize() {
    let mut state = State::Guards(BTreeSet::from([guard(0), guard(1)]));
    let other = State::Guards(BTreeSet::from([guard(1), guard(2)]));
    assert!(state.join(&other));
    let State::Guards(guards) = &state else {
        panic!()
    };
    assert_eq!(*guards, BTreeSet::from([guard(1)]));
    assert!(!state.join(&other));
}

#[test]
fn protected_choices_capture_each_throw_before_later_writes() {
    for protected in [false, true] {
        let mut graph = graph(
            (0..5)
                .map(|number| Op::Literal(Literal::Int(number)))
                .chain([read(), read(), Op::Parameter(0)]),
        );
        let blocks = vec![
            Block {
                events: vec![
                    Event::Throw,
                    Event::Choice {
                        left: 7,
                        right: 0,
                        op: CompareOp::Eq,
                        yes: vec![Event::Write(L0, 1), Event::Throw, Event::Write(L0, 2)],
                        no: vec![Event::Choice {
                            left: 7,
                            right: 1,
                            op: CompareOp::Eq,
                            yes: vec![Event::Write(L0, 3), Event::Throw, Event::Write(L0, 4)],
                            no: vec![Event::Write(L0, 4)],
                        }],
                    },
                    Event::Observe(5),
                ],
                exit: Exit::End,
            },
            Block {
                events: vec![Event::Observe(6)],
                exit: Exit::End,
            },
        ]
        .into();
        let mut ir = method(2);
        if protected {
            ir.blocks[B0].handlers.push(HandlerEdge {
                target: B1,
                catch_type: None,
                table_index: 0,
            });
        }
        graph.solve(
            &ir,
            &blocks,
            State::Definitions(vec![BTreeSet::from([0])].into()),
        );
        graph.solve(&ir, &blocks, State::Guards(BTreeSet::from([guard(7)])));

        let Op::Read { sources, .. } = &graph.nodes[5].op else {
            panic!()
        };
        assert_eq!(*sources, BTreeSet::from([2, 4]));
        assert_eq!(graph.nodes[5].guards, BTreeSet::from([guard(7)]));
        let Op::Read { sources, .. } = &graph.nodes[6].op else {
            panic!()
        };
        if protected {
            // Neither arm's write after its final throw reaches the handler.
            assert_eq!(*sources, BTreeSet::from([0, 1, 3]));
            assert_eq!(graph.nodes[6].guards, BTreeSet::from([guard(7)]));
            assert!(graph.nodes[6].reachable);
        } else {
            assert!(sources.is_empty());
            assert!(graph.nodes[6].guards.is_empty());
            assert!(!graph.nodes[6].reachable);
        }
    }
}

#[test]
fn writes_invalidate_choice_guards_before_later_throws() {
    let mut graph = graph([
        Op::Literal(Literal::Int(0)),
        Op::Parameter(0),
        read(),
        read(),
        read(),
        read(),
        read(),
        read(),
    ]);
    let blocks = vec![
        Block {
            events: vec![
                Event::Observe(2),
                Event::Choice {
                    left: 2,
                    right: 0,
                    op: CompareOp::Eq,
                    yes: vec![
                        Event::Observe(3),
                        Event::Throw,
                        Event::Write(L0, 0),
                        Event::Observe(4),
                        Event::Throw,
                    ],
                    no: vec![Event::Observe(5), Event::Throw],
                },
                Event::Observe(6),
            ],
            exit: Exit::End,
        },
        Block {
            events: vec![Event::Observe(7)],
            exit: Exit::End,
        },
    ]
    .into();
    let mut ir = method(2);
    ir.blocks[B0].handlers.push(HandlerEdge {
        target: B1,
        catch_type: None,
        table_index: 0,
    });
    graph.solve(
        &ir,
        &blocks,
        State::Definitions(vec![BTreeSet::from([1])].into()),
    );
    graph.solve(&ir, &blocks, State::Guards(BTreeSet::new()));

    assert_eq!(graph.known_at(3, 3), Some(0));
    assert!(graph.excludes_at(5, 5, 0));
    assert!(graph.nodes[4].guards.is_empty());
    assert!(graph.nodes[6].guards.is_empty());
    assert!(graph.nodes[7].guards.is_empty());
    for node in [6, 7] {
        let Op::Read { sources, .. } = &graph.nodes[node].op else {
            panic!()
        };
        assert_eq!(*sources, BTreeSet::from([0, 1]));
    }
}

#[test]
fn loop_reads_use_complete_definitions_and_keep_only_body_guards() {
    let mut graph = graph([
        Op::Literal(Literal::Int(0)),
        Op::Parameter(0),
        Op::Literal(Literal::Int(1)),
        read(),
        read(),
        read(),
    ]);
    let blocks = vec![
        Block {
            events: vec![Event::Observe(3)],
            exit: Exit::Branch {
                left: 3,
                right: 0,
                op: CompareOp::Eq,
                yes: B1,
                no: B2,
            },
        },
        Block {
            events: vec![Event::Observe(4), Event::Write(L0, 2)],
            exit: Exit::Goto(B0),
        },
        Block {
            events: vec![Event::Observe(5)],
            exit: Exit::End,
        },
    ]
    .into();
    let ir = method(3);
    graph.solve(
        &ir,
        &blocks,
        State::Definitions(vec![BTreeSet::from([1])].into()),
    );
    graph.solve(&ir, &blocks, State::Guards(BTreeSet::new()));

    for node in [3, 4, 5] {
        let Op::Read { sources, .. } = &graph.nodes[node].op else {
            panic!()
        };
        assert_eq!(*sources, BTreeSet::from([1, 2]));
    }
    assert!(graph.nodes[3].guards.is_empty());
    assert_eq!(graph.known_at(4, 4), Some(0));
    assert!(graph.excludes_at(5, 5, 0));
}

#[test]
fn shared_switch_targets_merge_guards_and_revisited_definitions() {
    let mut graph = graph([
        Op::Literal(Literal::Int(2)),
        Op::Parameter(0),
        read(),
        read(),
        read(),
    ]);
    let blocks = vec![
        Block {
            events: vec![Event::Observe(2)],
            exit: Exit::Switch {
                value: 2,
                cases: vec![(0, B1), (1, B1)],
                default: B2,
            },
        },
        Block {
            events: vec![Event::Observe(3), Event::Write(L0, 0)],
            exit: Exit::Goto(B0),
        },
        Block {
            events: vec![Event::Observe(4)],
            exit: Exit::End,
        },
    ]
    .into();
    let ir = method(3);
    graph.solve(
        &ir,
        &blocks,
        State::Definitions(vec![BTreeSet::from([1])].into()),
    );
    graph.solve(&ir, &blocks, State::Guards(BTreeSet::new()));

    for node in [2, 3, 4] {
        let Op::Read { sources, .. } = &graph.nodes[node].op else {
            panic!()
        };
        assert_eq!(*sources, BTreeSet::from([0, 1]));
    }
    assert!(graph.nodes[2].guards.is_empty());
    assert!(graph.nodes[3].guards.is_empty());
    assert_eq!(graph.known_at(3, 3), None);
    assert!(graph.excludes_at(4, 4, 0));
    assert!(graph.excludes_at(4, 4, 1));
}
