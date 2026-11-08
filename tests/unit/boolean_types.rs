use super::*;
use crate::index::test_ids::*;

fn fixture() -> RawIr {
    let mut raw = RawIr::new(MethodIr {
        entry: B0,
        blocks: vec![].into(),
        values: vec![].into(),
        locals: vec![].into(),
        parameters: vec![],
        return_type: JvmType::Void,
        initialization_sites: vec![],
    });
    value(&mut raw, ValueKind::Literal(Literal::Int(0)));
    value(&mut raw, ValueKind::Literal(Literal::Int(1)));
    for _ in 0..3 {
        value(&mut raw, phi(&[V0, V1]));
    }
    raw
}

fn phi(values: &[ValueId]) -> ValueKind {
    ValueKind::Phi {
        inputs: values
            .iter()
            .map(|&value| PhiInput {
                block: B0,
                throw_pc: None,
                value,
            })
            .collect(),
        slot: None,
    }
}

fn value(raw: &mut RawIr, kind: ValueKind) -> ValueId {
    raw.ir.values.push(Value {
        local: None,
        kind,
        ty: JvmType::Int,
        pc: 0,
        block: B0,
    })
}

fn terminator(raw: &mut RawIr, terminator: Terminator) {
    raw.ir.blocks.push(IrBlock {
        caught_exception: None,
        pc: 0,
        statements: vec![],
        terminator,
        handlers: vec![],
    });
    raw.events.push(vec![]);
}

fn comparison(raw: &mut RawIr, left: ValueId, op: CompareOp, right: ValueId) {
    terminator(
        raw,
        Terminator::Branch {
            condition: Condition { left, op, right },
            then_target: B0,
            else_target: B0,
        },
    );
}

fn call(raw: &mut RawIr, descriptor: &str, args: Vec<ValueId>) {
    value(
        raw,
        ValueKind::Call {
            kind: InvokeKind::Static,
            method: Member {
                owner: "Probe".into(),
                name: "use".into(),
                descriptor: descriptor.into(),
            },
            receiver: None,
            args,
        },
    );
}

fn types(raw: &mut RawIr, values: &[ValueId], expected: JvmType) {
    refine(raw, None);
    for &value in values {
        assert_eq!(raw.ir.values[value].ty, expected, "{value:?}");
    }
}

#[test]
fn equality_chains_recover_predicates_without_a_zero_test() {
    let mut raw = fixture();
    comparison(&mut raw, V2, CompareOp::Ne, V3);
    comparison(&mut raw, V3, CompareOp::Eq, V4);
    types(&mut raw, &[V2, V3, V4], JvmType::Boolean);
}

#[test]
fn copies_phi_cycles_and_eager_operators_share_the_logical_role() {
    let mut raw = fixture();
    let copied = value(&mut raw, ValueKind::Alias(V2));
    let eager = value(
        &mut raw,
        ValueKind::Binary {
            op: BinaryOp::Xor,
            left: copied,
            right: V4,
        },
    );
    raw.ir.values[V3].kind = phi(&[V0, eager]);
    raw.ir.values[V4].kind = phi(&[V1, V3]);
    comparison(&mut raw, eager, CompareOp::Ne, V2);
    types(&mut raw, &[V2, V3, V4, copied, eager], JvmType::Boolean);
}

#[test]
fn numeric_argument_blocks_every_connected_comparand_and_copy() {
    let mut raw = fixture();
    let copied = value(&mut raw, ValueKind::Alias(V2));
    comparison(&mut raw, copied, CompareOp::Ne, V3);
    comparison(&mut raw, V3, CompareOp::Eq, V4);
    comparison(&mut raw, V4, CompareOp::Ne, V0);
    call(&mut raw, "(I)V", vec![V2]);
    types(&mut raw, &[V2, V3, V4, copied], JvmType::Int);
}

#[test]
fn numeric_returns_ordering_switches_and_indices_keep_integer_locals() {
    for sink in 0..4 {
        let mut raw = fixture();
        comparison(&mut raw, V2, CompareOp::Eq, V3);
        comparison(&mut raw, V3, CompareOp::Ne, V0);
        match sink {
            0 => {
                raw.ir.return_type = JvmType::Int;
                terminator(&mut raw, Terminator::Return(Some(V2)));
            }
            1 => comparison(&mut raw, V2, CompareOp::Lt, V1),
            2 => terminator(
                &mut raw,
                Terminator::Switch {
                    value: V2,
                    cases: vec![(1, B0)],
                    default: B0,
                },
            ),
            _ => {
                let array = value(
                    &mut raw,
                    ValueKind::Parameter {
                        slot: 0,
                        index: Some(0),
                    },
                );
                value(
                    &mut raw,
                    ValueKind::ArrayLoad {
                        array,
                        index: V2,
                        element_type: JvmType::Int,
                    },
                );
            }
        }
        types(&mut raw, &[V2, V3], JvmType::Int);
    }
}

#[test]
fn boolean_descriptors_are_logical_sinks_for_paired_predicates() {
    for sink in 0..6 {
        let mut raw = fixture();
        comparison(&mut raw, V2, CompareOp::Ne, V3);
        comparison(&mut raw, V3, CompareOp::Eq, V4);
        // Only V2 has a descriptor demand. V3/V4 need the shared logical role;
        // treating the descriptor use as numeric would leave both as integers.
        let action = match sink {
            0 => {
                call(&mut raw, "(Z)V", vec![V2]);
                None
            }
            1 => Some(Statement::PutField {
                pc: Some(0),
                field: Member {
                    owner: "Probe".into(),
                    name: "flag".into(),
                    descriptor: "Z".into(),
                },
                receiver: None,
                value: V2,
            }),
            2 => Some(Statement::ArrayStore {
                array: V0,
                index: V1,
                value: V2,
                element_type: JvmType::Boolean,
            }),
            3 => {
                raw.ir.return_type = JvmType::Boolean;
                terminator(&mut raw, Terminator::Return(Some(V2)));
                None
            }
            4 => Some(Statement::ConstructorCall {
                pc: 0,
                receiver: V0,
                method: Member {
                    owner: "Probe".into(),
                    name: "<init>".into(),
                    descriptor: "(Z)V".into(),
                },
                args: vec![V2],
            }),
            _ => {
                value(
                    &mut raw,
                    ValueKind::New {
                        allocation_pc: 0,
                        class: "Probe".into(),
                        descriptor: "(Z)V".into(),
                        args: vec![V2],
                    },
                );
                None
            }
        };
        if let Some(action) = action {
            raw.events[B0].push(super::super::lift::Event {
                pc: 0,
                kind: EventKind::Action(action),
            });
        }
        types(&mut raw, &[V2, V3, V4], JvmType::Boolean);
    }
}

#[test]
fn shared_literals_do_not_join_independent_numeric_and_logical_locals() {
    let mut raw = fixture();
    comparison(&mut raw, V2, CompareOp::Ne, V0);
    comparison(&mut raw, V3, CompareOp::Eq, V1);
    call(&mut raw, "(III)V", vec![V0, V1, V4]);
    types(&mut raw, &[V2, V3], JvmType::Boolean);
    assert_eq!(raw.ir.values[V4].ty, JvmType::Int);
}

#[test]
fn noncanonical_boolean_input_can_participate_in_exact_equality() {
    let mut raw = fixture();
    let input = value(
        &mut raw,
        ValueKind::Parameter {
            slot: 0,
            index: Some(0),
        },
    );
    raw.ir.values[input].ty = JvmType::Boolean;
    raw.ir.values[V2].kind = phi(&[V0, input]);
    comparison(&mut raw, V2, CompareOp::Eq, V3);
    types(&mut raw, &[V2, V3], JvmType::Boolean);
}

#[test]
fn non_boolean_producer_contaminates_a_phi_cycle() {
    let mut raw = fixture();
    let two = value(&mut raw, ValueKind::Literal(Literal::Int(2)));
    raw.ir.values[V2].kind = phi(&[two, V3]);
    raw.ir.values[V3].kind = phi(&[V1, V2]);
    comparison(&mut raw, V2, CompareOp::Ne, V3);
    comparison(&mut raw, V3, CompareOp::Ne, V0);
    types(&mut raw, &[V2, V3], JvmType::Int);
}
