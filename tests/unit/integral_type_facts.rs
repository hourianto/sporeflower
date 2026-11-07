use super::*;
use crate::index::LocalId;
use crate::index::test_ids::*;
use crate::ir::ValueUse;
use crate::ir::{BinaryOp, CompareOp, Condition, IrBlock, Local, Statement, Terminator, Value};

fn fixture(start: i32, narrow: JvmType) -> MethodIr {
    let kinds = [
        ValueKind::Literal(Literal::Int(start)),
        ValueKind::Phi {
            inputs: vec![],
            slot: Some(1),
        },
        ValueKind::Literal(Literal::Int(1)),
        ValueKind::Binary {
            op: BinaryOp::Add,
            left: V1,
            right: V2,
        },
        ValueKind::Cast {
            value: V3,
            ty: narrow.clone(),
        },
        ValueKind::Parameter {
            slot: 0,
            index: Some(0),
        },
        ValueKind::InlineAssign {
            local: L0,
            value: V5,
        },
        ValueKind::Select {
            condition: Condition {
                op: CompareOp::Eq,
                left: V2,
                right: V2,
            },
            then_value: V6,
            else_value: V4,
        },
        ValueKind::Alias(V1),
    ];
    {
        let mut ir = MethodIr {
            entry: B0,
            blocks: vec![IrBlock {
                caught_exception: None,
                pc: 0,
                statements: vec![
                    Statement::Assign {
                        local: L0,
                        value: ValueUse::Reference(V0),
                    },
                    Statement::Assign {
                        local: L0,
                        value: ValueUse::Reference(V4),
                    },
                ],
                terminator: Terminator::Return(Some(V1)),
                handlers: vec![],
            }]
            .into(),
            values: kinds
                .into_iter()
                .enumerate()
                .map(|(id, kind)| Value {
                    local: None,
                    kind,
                    ty: if id == 4 {
                        narrow.clone()
                    } else {
                        JvmType::Int
                    },
                    pc: 0,
                    block: B0,
                })
                .collect(),
            locals: (0..2)
                .map(|id| Local {
                    name_hint: None,
                    name: format!("v{id}"),
                    ty: JvmType::Int,
                    parameter: (id == 1).then_some(0),
                    is_this: false,
                    needs_default: false,
                })
                .collect(),
            parameters: vec![V5],
            return_type: JvmType::Int,
            initialization_sites: vec![],
        };
        for (entity, metadata) in ir.values.iter_mut().zip(vec![
            None,
            Some(0),
            None,
            None,
            None,
            Some(1),
            None,
            None,
            None,
        ]) {
            entity.local = metadata.map(LocalId::from_index);
        }
        ir
    }
}

#[test]
fn narrow_update_storage_keeps_boundary_values_and_rejects_wider_initializers() {
    for (ty, boundary) in [
        (JvmType::Byte, 127),
        (JvmType::Short, 32767),
        (JvmType::Char, 65535),
    ] {
        for start in [-129, -128, -1, 0, 1, 127, 128, boundary, boundary + 1] {
            let mut ir = fixture(start, ty.clone());
            let expected = if IntegralRange::of_type(&ty).unwrap().min <= start && start <= boundary
            {
                ty.clone()
            } else {
                JvmType::Int
            };
            refine_integral_locals(&mut ir);
            assert_eq!(ir.locals[L0].ty, expected, "{ty:?} at {start}");
            assert_eq!(ir.values[V1].ty, JvmType::Int, "SSA type is unchanged");
            assert!(matches!(&ir.values[V4].kind, ValueKind::Cast { ty: cast, .. } if cast == &ty));
        }
    }
}

#[test]
fn every_physical_write_including_terminal_conditional_assignments_must_fit() {
    let mut ir = fixture(0, JvmType::Byte);
    ir.blocks[B0].terminator = Terminator::Return(Some(V7));
    refine_integral_locals(&mut ir);
    assert_eq!(ir.locals[L0].ty, JvmType::Int);
    let mut mixed = fixture(0, JvmType::Byte);
    mixed.blocks[B0].statements.push(Statement::Assign {
        local: L0,
        value: ValueUse::Reference(V3),
    });
    refine_integral_locals(&mut mixed);
    assert_eq!(
        mixed.locals[L0].ty,
        JvmType::Int,
        "an unconverted update remains full int"
    );
}

#[test]
fn copy_cycles_need_a_real_range_anchor_and_arbitrary_parameter_slots_stay_wide() {
    let mut ir = fixture(0, JvmType::Byte);
    ir.blocks[B0].statements.push(Statement::Assign {
        local: L0,
        value: ValueUse::Reference(V8),
    });
    refine_integral_locals(&mut ir);
    assert_eq!(ir.locals[L0].ty, JvmType::Byte);
    for descriptor in [
        JvmType::Byte,
        JvmType::Short,
        JvmType::Char,
        JvmType::Boolean,
    ] {
        let mut ir = fixture(0, JvmType::Byte);
        ir.values[V5].ty = descriptor.clone();
        ir.locals[L1].ty = descriptor;
        ir.blocks[B0].statements.push(Statement::Assign {
            local: L0,
            value: ValueUse::Reference(V5),
        });
        refine_integral_locals(&mut ir);
        assert_eq!(ir.locals[L0].ty, JvmType::Int);
    }
    let mut unanchored = fixture(0, JvmType::Byte);
    unanchored.blocks[B0].statements = vec![Statement::Assign {
        local: L0,
        value: ValueUse::Reference(V8),
    }];
    refine_integral_locals(&mut unanchored);
    assert_eq!(unanchored.locals[L0].ty, JvmType::Int);
}

#[test]
fn a_constant_bound_to_mutable_storage_is_a_read_not_a_constant_definition() {
    let mut ir = fixture(0, JvmType::Byte);
    ir.values[V0].local = Some(L1);
    refine_integral_locals(&mut ir);
    assert_eq!(ir.locals[L0].ty, JvmType::Int);
}

#[test]
fn shared_graph_records_scheduled_writes_and_distinguishes_assignment_roots() {
    let mut ir = fixture(0, JvmType::Byte);
    ir.values[V0].local = Some(L0);
    ir.blocks[B0].statements[0] = Statement::Assign {
        local: L0,
        value: ValueUse::Definition(V0),
    };
    let local = ir.values.len() + L0.index();
    let graph = ValueGraph::new(&ir);
    assert_eq!(graph.inputs[local], vec![V0.index(), V4.index()]);
    assert_eq!(ValueGraph::node(&ir, ValueUse::Reference(V0)), local);
    assert_eq!(ValueGraph::node(&ir, ValueUse::Definition(V0)), V0.index());

    // The dormant embedded assignment above does not write until evaluated by
    // this select. Its raw parameter remains a binding even at a definition root.
    ir.blocks[B0].terminator = Terminator::Return(Some(V7));
    let graph = ValueGraph::new(&ir);
    assert_eq!(
        graph.inputs[local],
        vec![V0.index(), V4.index(), ir.values.len() + L1.index()]
    );
}

#[test]
fn coalesced_incoming_bindings_and_unwritten_defaults_stay_full_width() {
    let mut ir = fixture(0, JvmType::Byte);
    ir.locals[L1].parameter = None;
    ir.locals[L1].needs_default = true;
    ir.values[V5].ty = JvmType::Byte;
    ir.blocks[B0].statements.push(Statement::Assign {
        local: L1,
        value: ValueUse::Reference(V4),
    });
    refine_integral_locals(&mut ir);
    assert_eq!(ir.locals[L1].ty, JvmType::Int);

    ir.blocks[B0].statements.clear();
    ir.locals[L0].needs_default = true;
    let facts = IntegralFacts::analyze(&ir);
    assert_eq!(facts.local_range(L0), Some(IntegralRange::INT));
}
