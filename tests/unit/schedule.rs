use super::*;
use crate::index::test_ids::*;

fn capture_fixture(kinds: Vec<(ValueKind, Option<LocalId>)>) -> MethodIr {
    MethodIr {
        entry: B0,
        values: kinds
            .into_iter()
            .enumerate()
            .map(|(pc, (kind, local))| Value {
                local,
                kind,
                ty: JvmType::Int,
                pc: pc as u32,
                block: B0,
            })
            .collect(),
        blocks: vec![].into(),
        locals: (0..2)
            .map(|i| Local {
                name_hint: None,
                name: format!("v{i}"),
                ty: JvmType::Int,
                parameter: None,
                is_this: false,
                needs_default: false,
            })
            .collect(),
        parameters: vec![],
        return_type: JvmType::Int,
        initialization_sites: vec![],
    }
}

#[test]
fn captures_cross_deep_materialized_dependencies_without_call_stack_growth() {
    const DEPTH: usize = 32_768;
    let mut kinds = vec![(
        ValueKind::Parameter {
            slot: 0,
            index: Some(0),
        },
        Some(L0),
    )];
    let mut pending = crate::hash::HashSet::default();
    for i in 1..=DEPTH {
        // Every source expression has bounded depth, but capturing its
        // materialized dependencies still walks the entire chain.
        let local = (i % MAX_EXPRESSION_DEPTH == 0).then_some(L1);
        if local.is_some() {
            pending.insert(ValueId::from_index(i));
        }
        kinds.push((
            ValueKind::Unary {
                op: UnaryOp::Neg,
                value: ValueId::from_index(i - 1),
            },
            local,
        ));
    }
    let mut ir = capture_fixture(kinds);
    let expected = pending.clone();
    let mut removed = crate::hash::HashSet::default();
    let uses = vec![1; ir.values.len()].into();
    let mut value = capture_value(
        &mut ir,
        ValueId::from_index(DEPTH),
        &mut pending,
        &mut removed,
        &uses,
    );

    assert!(pending.is_empty());
    assert_eq!(removed, expected);
    for pc in (1..=DEPTH).rev() {
        let expression = &ir.values[value];
        assert_eq!(expression.pc, pc as u32);
        assert!(expression.local.is_none());
        let ValueKind::Unary {
            op: UnaryOp::Neg,
            value: operand,
        } = expression.kind
        else {
            panic!("capture changed a dependency's operation");
        };
        value = operand;
    }
    assert_eq!(value, V0);
    assert_eq!(ir.values[V0].local, Some(L0));
    for original in removed {
        assert_eq!(ir.values[original].local, Some(L1));
    }
}

#[test]
fn captures_do_not_evaluate_phi_backedges_or_steal_their_definitions() {
    for local in [None, Some(L1)] {
        let mut ir = capture_fixture(vec![
            (ValueKind::Literal(Literal::Int(7)), None),
            (
                ValueKind::Phi {
                    inputs: vec![
                        PhiInput {
                            block: B0,
                            throw_pc: None,
                            value: V0,
                        },
                        PhiInput {
                            block: B1,
                            throw_pc: None,
                            value: V2,
                        },
                    ],
                    slot: Some(0),
                },
                local,
            ),
            (
                ValueKind::Unary {
                    op: UnaryOp::Neg,
                    value: V1,
                },
                Some(L0),
            ),
            (
                ValueKind::Binary {
                    op: BinaryOp::Add,
                    left: V2,
                    right: V1,
                },
                None,
            ),
        ]);
        let mut pending = [V2].into_iter().collect();
        let mut removed = crate::hash::HashSet::default();
        let uses = vec![1, 2, 1, 1].into();

        assert_eq!(
            capture_value(&mut ir, V1, &mut pending, &mut removed, &uses),
            V1
        );
        assert_eq!(pending, [V2].into_iter().collect());
        assert!(removed.is_empty());
        assert_eq!(ir.values.len(), 4);

        let captured = capture_value(&mut ir, V3, &mut pending, &mut removed, &uses);
        assert!(pending.is_empty());
        assert_eq!(removed, [V2].into_iter().collect());
        let ValueKind::Binary { left, right, .. } = ir.values[captured].kind else {
            panic!("expected rebuilt addition");
        };
        assert_eq!(right, V1);
        assert!(matches!(
            ir.values[left].kind,
            ValueKind::Unary {
                op: UnaryOp::Neg,
                value: V1,
            }
        ));
        assert_eq!(ir.values[V1].local, local);
        assert_eq!(ir.values[V2].local, Some(L0));
    }
}

#[test]
fn captures_keep_first_use_order_and_original_storage_for_later_reads() {
    let call = |name: &str, args| ValueKind::Call {
        kind: InvokeKind::Static,
        method: Member {
            owner: "Input".into(),
            name: name.into(),
            descriptor: "(I)I".into(),
        },
        receiver: None,
        args,
    };
    let mut ir = capture_fixture(vec![
        (ValueKind::Literal(Literal::Int(7)), None),
        (call("first", vec![V0]), Some(L0)),
        (call("second", vec![V1]), Some(L1)),
        (
            ValueKind::Binary {
                op: BinaryOp::Add,
                left: V2,
                right: V1,
            },
            None,
        ),
    ]);
    let mut pending = [V1, V2].into_iter().collect();
    let mut removed = crate::hash::HashSet::default();
    let captured = capture_value(
        &mut ir,
        V3,
        &mut pending,
        &mut removed,
        &vec![1, 2, 1, 1].into(),
    );

    assert!(pending.is_empty());
    assert_eq!(removed, [V1, V2].into_iter().collect());
    let ValueKind::Binary { left, right, .. } = ir.values[captured].kind else {
        panic!("expected rebuilt addition");
    };
    assert_eq!(right, V1);
    let ValueKind::Call { method, args, .. } = &ir.values[left].kind else {
        panic!("single-use capture must not retain a local assignment");
    };
    assert_eq!(method.name, "second");
    let ValueKind::InlineAssign { local, value } = ir.values[args[0]].kind else {
        panic!("shared capture must write before its later read");
    };
    assert_eq!(local, L0);
    assert!(ir.values[value].local.is_none());
    let ValueKind::Call { method, args, .. } = &ir.values[value].kind else {
        panic!("expected captured call definition");
    };
    assert_eq!(method.name, "first");
    assert_eq!(args, &[V0]);
    assert_eq!(ir.values[V1].local, Some(L0));
    assert_eq!(ir.values[V2].local, Some(L1));
    assert_eq!(
        capture_value(
            &mut ir,
            V1,
            &mut pending,
            &mut removed,
            &vec![1, 2, 1, 1].into()
        ),
        V1
    );
}

#[test]
fn monitor_copy_split_preserves_original_and_synthetic_statement_origins() {
    let mut raw = RawIr::new(MethodIr {
        entry: B0,
        values: vec![Value {
            local: None,
            kind: ValueKind::Literal(Literal::Null),
            ty: JvmType::Object("java/lang/Object".into()),
            pc: 9,
            block: B0,
        }]
        .into(),
        blocks: (0..2)
            .map(|_| IrBlock {
                caught_exception: None,
                pc: 3,
                statements: vec![],
                terminator: Terminator::Goto(B1),
                handlers: vec![],
            })
            .collect(),
        locals: vec![Local {
            name_hint: None,
            name: "copy".into(),
            ty: JvmType::Object("java/lang/Object".into()),
            parameter: None,
            is_this: false,
            needs_default: false,
        }]
        .into(),
        parameters: vec![],
        return_type: JvmType::Void,
        initialization_sites: vec![],
    });
    raw.ir.blocks[B0].handlers.push(crate::cfg::HandlerEdge {
        target: B1,
        catch_type: None,
        table_index: 0,
    });
    raw.set_statements(
        B0,
        vec![
            (
                Statement::Monitor {
                    value: V0,
                    enter: true,
                },
                7,
            ),
            (
                Statement::Assign {
                    local: L0,
                    value: ValueUse::Reference(V0),
                },
                9,
            ),
            (
                Statement::Assign {
                    local: L0,
                    value: ValueUse::Reference(V0),
                },
                u32::MAX,
            ),
        ],
    );

    split_monitor_copies(&mut raw);

    raw.assert_statement_origins();
    assert_eq!(raw.ir.blocks.len(), 3);
    assert_eq!(raw.statement_pcs(B0), &[7]);
    assert_eq!(raw.statement_pcs(B2), &[9, u32::MAX]);
    assert!(matches!(raw.ir.blocks[B0].terminator, Terminator::Goto(B2)));
    assert!(matches!(raw.ir.blocks[B2].terminator, Terminator::Goto(B1)));
    assert_eq!(raw.ir.blocks[B0].handlers.len(), 1);
    assert!(raw.ir.blocks[B2].handlers.is_empty());
}

#[test]
fn precomputed_choice_arms_capture_enclosing_calls_and_keep_existing_boundaries() {
    use super::lift::Event;

    let mut ir = capture_fixture(vec![
        (
            ValueKind::Parameter {
                slot: 0,
                index: Some(0),
            },
            Some(L0),
        ),
        (ValueKind::Literal(Literal::Int(0)), None),
    ]);
    let mut roots = Vec::new();
    for length in [9, 5] {
        let mut argument = V1;
        for _ in 0..length {
            argument = ir.values.push(Value {
                kind: ValueKind::Call {
                    kind: InvokeKind::Static,
                    method: Member {
                        owner: "Input".into(),
                        name: "step".into(),
                        descriptor: "(I)I".into(),
                    },
                    receiver: None,
                    args: vec![argument],
                },
                ty: JvmType::Int,
                local: None,
                pc: ir.values.len() as u32,
                block: B0,
            });
        }
        roots.push(argument);
    }
    let choice = ir.values.push(Value {
        kind: ValueKind::Select {
            condition: Condition {
                left: V0,
                op: CompareOp::Eq,
                right: V1,
            },
            then_value: roots[0],
            else_value: roots[1],
        },
        ty: JvmType::Int,
        local: None,
        pc: ir.values.len() as u32,
        block: B0,
    });
    ir.blocks.push(IrBlock {
        pc: 0,
        statements: vec![],
        terminator: Terminator::Return(Some(choice)),
        handlers: vec![],
        caught_exception: None,
    });
    let mut raw = RawIr::new(ir);
    raw.events[B0] = raw
        .ir
        .values
        .ids()
        .skip(2)
        .map(|value| Event {
            pc: raw.ir.values[value].pc,
            kind: EventKind::Define(value),
        })
        .collect();
    // Both chains were evaluated before the branch; the lazy choice owns no
    // calls. They must remain unconditional, with each chain kept together.
    raw.conditional_regions
        .insert(choice, [Vec::new(), Vec::new()]);
    let live = vec![true; raw.ir.values.len()].into();
    let uses = vec![1; raw.ir.values.len()].into();
    let snapshots = HashMap::default();
    let conditional_values = crate::hash::HashSet::default();
    let order = EvaluationOrder::new(&raw, &live, &uses, &snapshots, &conditional_values);
    for existing in [None, Some(V3)] {
        let mut materialize: IdVec<_, _> = vec![false; raw.ir.values.len()].into();
        if let Some(value) = existing {
            materialize[value] = true;
        }
        order.enforce(&mut materialize);
        assert_eq!(
            materialize
                .ids()
                .filter(|&value| materialize[value])
                .collect::<Vec<_>>(),
            existing
                .into_iter()
                .chain(roots.iter().copied())
                .collect::<Vec<_>>()
        );
    }
}
