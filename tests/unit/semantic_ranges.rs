use super::*;
use crate::ir::{
    BlockId, CompareOp, Condition, IrBlock, Literal, Local, MethodIr, Statement, Terminator, Value,
    ValueId, ValueKind,
};
use crate::mappings::DomainId;

fn int(number: i32) -> (JvmType, Op) {
    (JvmType::Int, Op::Literal(Literal::Int(number)))
}
fn binary(op: BinaryOp, left: NodeId, right: NodeId) -> (JvmType, Op) {
    (JvmType::Int, Op::Binary { op, left, right })
}
fn layout(stride: u32, offset: u32, planes: bool) -> RecordLayout {
    RecordLayout {
        domain: DomainId(0),
        stride,
        offset,
        planes,
    }
}

#[test]
fn shared_arithmetic_paths_retain_their_precise_range() {
    let mut ops = vec![
        (JvmType::Int, Op::Parameter(0)),
        (
            JvmType::Byte,
            Op::Cast {
                value: 0,
                ty: JvmType::Byte,
            },
        ),
        int(2),
    ];
    let mut value = 1;
    // The expression DAG is linear, but expanding both inputs of every
    // addition as separate trees would require 2^40 recursive proofs.
    for _ in 0..40 {
        let sum = ops.len();
        ops.push(binary(BinaryOp::Add, value, value));
        value = ops.len();
        ops.push(binary(BinaryOp::Div, sum, 2));
    }
    let graph = test_graph(ops);
    assert_eq!(range(&graph, value), Range::full(&JvmType::Byte));
}

#[test]
fn branching_narrow_counter_checks_bounds_without_expanding_cycle_paths() {
    let branches = 16;
    let mut ops = vec![int(0), int(1)];
    let sources: BTreeSet<_> = std::iter::once(0)
        .chain((0..branches).map(|branch| 5 + branch * 4))
        .collect();
    for _ in 0..branches {
        let read = ops.len();
        ops.push((
            JvmType::Short,
            Op::Read {
                local: LocalId::from_index(0),
                sources: sources.clone(),
            },
        ));
        ops.push(binary(BinaryOp::Add, read, 1));
        ops.push((
            JvmType::Short,
            Op::Cast {
                value: read + 1,
                ty: JvmType::Short,
            },
        ));
        // Aliases must retain the operation's bound as well.
        ops.push((JvmType::Short, Op::Copy(read + 2)));
    }
    let graph = test_graph(ops);
    let queries = Queries::new(&graph);
    assert_eq!(queries.range(2), Range::full(&JvmType::Short));
    assert_eq!(
        queries.range(3),
        Range {
            min: i64::from(i16::MIN) + 1,
            max: i64::from(i16::MAX) + 1,
        }
    );
    assert_eq!(queries.range(4), Range::full(&JvmType::Short));
    // The read's cyclic fallback cannot poison a query entered through the
    // addition, nor may that addition's wider bound contaminate the read.
    assert_eq!(queries.range(2), Range::full(&JvmType::Short));
    assert_eq!(queries.range(3).max, i64::from(i16::MAX) + 1);
    assert!(queries.induction[2].is_none(), "narrowing updates can wrap");
}

#[test]
fn raw_narrow_parameter_needs_a_real_narrowing_operation() {
    let graph = test_graph(vec![
        (JvmType::Byte, Op::Parameter(0)),
        (
            JvmType::Byte,
            Op::Cast {
                value: 0,
                ty: JvmType::Byte,
            },
        ),
    ]);
    assert_eq!(range(&graph, 0), Range::full(&JvmType::Int));
    assert_eq!(
        range(&graph, 1),
        Range {
            min: -128,
            max: 127
        }
    );
}

#[test]
fn unsigned_long_shifts_use_all_six_distance_bits() {
    let graph = test_graph(vec![
        (JvmType::Long, Op::Parameter(0)),
        int(40),
        (
            JvmType::Long,
            Op::Binary {
                op: BinaryOp::Ushr,
                left: 0,
                right: 1,
            },
        ),
        (
            JvmType::Long,
            Op::Binary {
                op: BinaryOp::Shl,
                left: 0,
                right: 1,
            },
        ),
        (
            JvmType::Int,
            Op::Cast {
                value: 3,
                ty: JvmType::Int,
            },
        ),
        int(1),
        binary(BinaryOp::Add, 4, 5),
    ]);
    assert_eq!(
        range(&graph, 2),
        Range {
            min: 0,
            max: 0xff_ffff
        }
    );
    assert_eq!(
        slot(&Queries::new(&graph), 6, &layout(1024, 0, false)),
        Some(1)
    );
}

#[test]
fn non_power_of_two_records_require_nonwrapping_address_arithmetic() {
    let graph = test_graph(vec![
        (JvmType::Int, Op::Parameter(0)),
        int(255),
        binary(BinaryOp::And, 0, 1),
        int(3),
        binary(BinaryOp::Mul, 2, 3),
        int(1),
        binary(BinaryOp::Add, 4, 5),
        binary(BinaryOp::Mul, 0, 3),
        binary(BinaryOp::Add, 7, 5),
    ]);
    assert_eq!(
        slot(&Queries::new(&graph), 6, &layout(3, 0, false)),
        Some(1)
    );
    assert_eq!(slot(&Queries::new(&graph), 8, &layout(3, 0, false)), None);
    assert_eq!(slot(&Queries::new(&graph), 6, &layout(3, 5, false)), None);
}

#[test]
fn cyclic_record_residues_check_every_seed_and_backedge_without_giving_bounds() {
    let local = LocalId::from_index(0);
    let read = || {
        (
            JvmType::Int,
            Op::Read {
                local,
                sources: BTreeSet::from([0, 1, 6, 8]),
            },
        )
    };
    let mut graph = test_graph(vec![
        int(0),
        int(16),
        int(8),
        int(16),
        read(),
        (JvmType::Int, Op::Copy(4)),
        binary(BinaryOp::Add, 5, 2),
        read(),
        binary(BinaryOp::Sub, 7, 3),
        int(1),
        binary(BinaryOp::Add, 4, 9),
    ]);
    assert_eq!(
        slot(&Queries::new(&graph), 10, &layout(8, 0, false)),
        Some(1)
    );
    let captured = graph.nodes.len();
    let mut capture = graph.nodes[4].clone();
    capture.op = Op::Read {
        local: LocalId::from_index(1),
        sources: BTreeSet::from([10]),
    };
    graph.nodes.push(capture);
    assert_eq!(
        offset_literal(&Queries::new(&graph), captured, &layout(8, 0, false), 1),
        Some(9)
    );
    graph.nodes[captured].op = Op::Read {
        local: LocalId::from_index(1),
        sources: BTreeSet::from([9, 10]),
    };
    assert_eq!(
        offset_literal(&Queries::new(&graph), captured, &layout(8, 0, false), 1),
        None
    );
    graph.nodes[captured].op = Op::Copy(captured);
    assert_eq!(
        offset_literal(&Queries::new(&graph), captured, &layout(8, 0, false), 1),
        None
    );
    assert_eq!(range(&graph, 4), Range::full(&JvmType::Int));
    assert!(induction(&graph, 4).is_none());

    graph.nodes[1].op = int(1).1;
    assert_eq!(slot(&Queries::new(&graph), 10, &layout(8, 0, false)), None);
    graph.nodes[1].op = Op::Unknown;
    assert_eq!(slot(&Queries::new(&graph), 10, &layout(8, 0, false)), None);
    graph.nodes[1].op = int(16).1;
    graph.nodes[3].op = int(15).1;
    assert_eq!(slot(&Queries::new(&graph), 10, &layout(8, 0, false)), None);

    graph.nodes[3].op = int(16).1;
    graph.nodes[7].op = Op::Read {
        local,
        sources: BTreeSet::from([9]),
    };
    assert_eq!(
        slot(&Queries::new(&graph), 10, &layout(8, 0, false)),
        None,
        "another read of the same local has different reaching definitions"
    );
}

#[test]
fn cyclic_record_residues_respect_arithmetic_and_narrowing_wrap_widths() {
    for (ty, storage, start, step, stride, expected) in [
        (
            JvmType::Int,
            JvmType::Int,
            i64::from(i32::MAX) - 6,
            8,
            8,
            Some(1),
        ),
        (JvmType::Long, JvmType::Long, i64::MAX - 6, 8, 8, Some(1)),
        (JvmType::Int, JvmType::Byte, 121, 8, 8, Some(1)),
        (JvmType::Int, JvmType::Short, 32761, 8, 8, Some(1)),
        (JvmType::Int, JvmType::Char, 65529, 8, 8, Some(1)),
        (
            JvmType::Long,
            JvmType::Int,
            i64::from(i32::MAX) - 6,
            8,
            8,
            Some(1),
        ),
        (JvmType::Int, JvmType::Byte, 1, 512, 512, None),
        (JvmType::Int, JvmType::Short, 1, 131072, 131072, None),
        (
            JvmType::Int,
            JvmType::Int,
            i64::from(i32::MAX) - 1,
            3,
            3,
            None,
        ),
        (JvmType::Long, JvmType::Long, i64::MAX - 1, 3, 3, None),
        (JvmType::Int, JvmType::Byte, 126, 3, 3, None),
    ] {
        let literal = |number| {
            (
                ty.clone(),
                Op::Literal(if ty == JvmType::Long {
                    Literal::Long(number)
                } else {
                    Literal::Int(number as i32)
                }),
            )
        };
        let graph = test_graph(vec![
            literal(start),
            literal(step),
            (
                ty.clone(),
                Op::Read {
                    local: LocalId::from_index(0),
                    sources: BTreeSet::from([0, 4]),
                },
            ),
            (
                ty.clone(),
                Op::Binary {
                    op: BinaryOp::Add,
                    left: 2,
                    right: 1,
                },
            ),
            (
                storage.clone(),
                Op::Cast {
                    value: 3,
                    ty: storage.clone(),
                },
            ),
        ]);
        assert_eq!(
            residue(&Queries::new(&graph), 2, stride, &mut HashSet::default()),
            expected,
            "{ty:?} to {storage:?}, start {start}, step {step}, stride {stride}"
        );
    }
}

#[test]
fn cyclic_record_residues_need_an_independent_seed() {
    let graph = test_graph(vec![
        int(8),
        (
            JvmType::Int,
            Op::Read {
                local: LocalId::from_index(0),
                sources: BTreeSet::from([2]),
            },
        ),
        binary(BinaryOp::Add, 1, 0),
    ]);
    assert_eq!(
        residue(&Queries::new(&graph), 1, 8, &mut HashSet::default()),
        None
    );
}

#[test]
fn plane_projection_proves_the_full_address_and_names_separate_header_bases() {
    let graph = test_graph(vec![
        (JvmType::Int, Op::Parameter(0)),
        int(3),
        binary(BinaryOp::And, 0, 1),
        int(1),
        binary(BinaryOp::Add, 3, 2),
        int(4),
        binary(BinaryOp::Add, 4, 5),
        int(5),
        binary(BinaryOp::Add, 7, 0),
    ]);
    let layout = layout(4, 1, true);
    assert_eq!(slot(&Queries::new(&graph), 6, &layout), Some(1));
    assert!(plane_offsets(&graph, 6, &layout).contains(&(5, 1, 0)));
    assert_eq!(slot(&Queries::new(&graph), 8, &layout), None);
    assert!(plane_offsets(&graph, 8, &layout).contains(&(7, 1, 1)));
}

fn counter_ir(
    start: i32,
    step: i32,
    bound: i32,
    comparison: CompareOp,
    storage: JvmType,
) -> MethodIr {
    let b = |id| BlockId::from_index(id);
    let v = |id| ValueId::from_index(id);
    let local = LocalId::from_index(0);
    let value = |kind, local, ty, block| Value {
        local,
        kind,
        ty,
        pc: 0,
        block: b(block),
    };
    let mut values = vec![
        value(
            ValueKind::Literal(Literal::Int(start)),
            Some(local),
            JvmType::Int,
            0,
        ),
        value(
            ValueKind::Literal(Literal::Int(bound)),
            None,
            JvmType::Int,
            1,
        ),
        value(
            ValueKind::Literal(Literal::Int(step)),
            None,
            JvmType::Int,
            2,
        ),
        value(
            ValueKind::Binary {
                op: BinaryOp::Add,
                left: v(0),
                right: v(2),
            },
            None,
            JvmType::Int,
            2,
        ),
    ];
    let update = if storage == JvmType::Int {
        values[3].local = Some(local);
        v(3)
    } else {
        values.push(value(
            ValueKind::Cast {
                value: v(3),
                ty: storage.clone(),
            },
            Some(local),
            storage.clone(),
            2,
        ));
        v(4)
    };
    let body = v(values.len());
    values.push(value(ValueKind::Alias(v(0)), None, JvmType::Int, 2));
    let block = |statements, terminator| IrBlock {
        caught_exception: None,
        pc: 0,
        statements,
        terminator,
        handlers: Vec::new(),
    };
    MethodIr {
        entry: b(0),
        values: values.into(),
        blocks: vec![
            block(
                vec![Statement::Assign {
                    local,
                    value: crate::ir::ValueUse::Definition(v(0)),
                }],
                Terminator::Goto(b(1)),
            ),
            block(
                Vec::new(),
                Terminator::Branch {
                    condition: Condition {
                        left: v(0),
                        right: v(1),
                        op: comparison,
                    },
                    then_target: b(2),
                    else_target: b(3),
                },
            ),
            block(
                vec![
                    Statement::Eval(body),
                    Statement::Assign {
                        local,
                        value: crate::ir::ValueUse::Definition(update),
                    },
                ],
                Terminator::Goto(b(1)),
            ),
            block(Vec::new(), Terminator::Return(Some(v(0)))),
        ]
        .into(),
        locals: vec![Local {
            name_hint: None,
            name: "i".into(),
            ty: storage,
            parameter: None,
            is_this: false,
            needs_default: false,
        }]
        .into(),
        parameters: Vec::new(),
        return_type: JvmType::Int,
        initialization_sites: Vec::new(),
    }
}

fn counter_graph(
    start: i32,
    step: i32,
    bound: i32,
    comparison: CompareOp,
    storage: JvmType,
) -> Graph {
    Graph::build(&counter_ir(start, step, bound, comparison, storage))
}

#[test]
fn induction_is_body_only_and_preserves_record_alignment() {
    let mut graph = counter_graph(1, 3, 20, CompareOp::Lt, JvmType::Int);
    let reads: Vec<_> = graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| matches!(node.op, Op::Read { .. }))
        .collect();
    assert!(
        reads
            .iter()
            .any(|(node, _)| induction(&graph, *node).is_some())
    );
    for (node, data) in reads {
        let proof = induction(&graph, node);
        assert_eq!(proof.is_some(), data.site.block == BlockId::from_index(2));
        if let Some(proof) = proof {
            assert_eq!(proof.range, Range { min: 1, max: 19 });
            assert_eq!(
                slot(&Queries::new(&graph), node, &layout(3, 0, false)),
                Some(1)
            );
        }
    }
    let mut later_bound = graph
        .nodes
        .iter()
        .flat_map(|node| &node.guards)
        .find(|guard| guard.relation == 2)
        .unwrap()
        .clone();
    later_bound.number = 100;
    let exit_read = graph
        .nodes
        .iter()
        .position(|node| {
            node.site.block == BlockId::from_index(3) && matches!(node.op, Op::Read { .. })
        })
        .unwrap();
    graph.nodes[exit_read].guards.insert(later_bound);
    assert!(
        induction(&graph, exit_read).is_none(),
        "a new post-loop range test must not import the loop body's domain"
    );
}

#[test]
fn induction_checks_narrow_storage_and_signed_int_wraparound() {
    for (start, step, bound, comparison, storage, expected) in [
        (0, 1, 10, CompareOp::Lt, JvmType::Byte, true),
        (0, 1, 127, CompareOp::Le, JvmType::Byte, false),
        (0, 1, i32::MAX, CompareOp::Lt, JvmType::Int, true),
        (0, 1, i32::MAX, CompareOp::Le, JvmType::Int, false),
        (7, -2, 0, CompareOp::Ge, JvmType::Short, true),
        (7, -2, 0, CompareOp::Ge, JvmType::Char, false),
    ] {
        let graph = counter_graph(start, step, bound, comparison, storage.clone());
        let actual = graph
            .nodes
            .iter()
            .enumerate()
            .any(|(node, _)| induction(&graph, node).is_some());
        assert_eq!(
            actual, expected,
            "{storage:?}: start {start}, step {step}, bound {bound}"
        );
    }
}

#[test]
fn variable_strict_bounds_prove_only_nonwrapping_progressions() {
    for (start, step, comparison, storage, expected) in [
        (0, 1, CompareOp::Lt, JvmType::Int, true),
        (0, 1, CompareOp::Le, JvmType::Int, false),
        (0, 2, CompareOp::Lt, JvmType::Int, false),
        (1, 2, CompareOp::Lt, JvmType::Int, true),
        (0, 1, CompareOp::Lt, JvmType::Byte, false),
    ] {
        let mut ir = counter_ir(start, step, 0, comparison, storage.clone());
        let bound = ValueId::from_index(1);
        ir.values[bound].kind = ValueKind::Parameter {
            slot: 0,
            index: Some(0),
        };
        // A declared byte bound still occupies an unconstrained JVM int.
        ir.values[bound].ty = JvmType::Byte;
        ir.parameters.push(bound);
        let graph = Graph::build(&ir);
        let actual = graph
            .nodes
            .iter()
            .enumerate()
            .any(|(node, _)| induction(&graph, node).is_some());
        assert_eq!(
            actual, expected,
            "{storage:?}: start {start}, step {step}, {comparison:?}"
        );
    }
}

#[test]
fn array_length_strict_bound_allows_an_int_counter() {
    let mut ir = counter_ir(0, 1, 0, CompareOp::Lt, JvmType::Int);
    let array = ir.values.push(Value {
        local: None,
        kind: ValueKind::Parameter {
            slot: 0,
            index: Some(0),
        },
        ty: JvmType::Array(Box::new(JvmType::Int)),
        pc: 0,
        block: BlockId::from_index(0),
    });
    ir.parameters.push(array);
    ir.values[ValueId::from_index(1)].kind = ValueKind::ArrayLength { array };
    let graph = Graph::build(&ir);
    assert!(
        graph
            .nodes
            .iter()
            .enumerate()
            .any(|(node, _)| induction(&graph, node).is_some())
    );
}
