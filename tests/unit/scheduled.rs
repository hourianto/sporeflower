use super::*;
use crate::classfile::JvmType;
use crate::index::LocalId;
use crate::index::test_ids::*;
use crate::ir::{InvokeKind, Local, Member, PhiInput, Value};

fn fixture(kinds: Vec<ValueKind>, bindings: Vec<Option<LocalId>>) -> MethodIr {
    {
        let mut ir = MethodIr {
            entry: B0,
            values: kinds
                .into_iter()
                .map(|kind| Value {
                    local: None,
                    kind,
                    ty: JvmType::Int,
                    pc: 0,
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
        };
        for (entity, metadata) in ir.values.iter_mut().zip(bindings) {
            entity.local = metadata;
        }
        ir
    }
}

fn call() -> ValueKind {
    ValueKind::Call {
        kind: InvokeKind::Static,
        method: Member {
            owner: "Input".into(),
            name: "read".into(),
            descriptor: "()I".into(),
        },
        receiver: None,
        args: vec![],
    }
}

#[test]
fn capture_evaluates_the_definition_even_when_its_value_has_a_local() {
    let ir = fixture(
        vec![
            call(),
            ValueKind::InlineAssign {
                local: L1,
                value: V0,
            },
        ],
        vec![Some(L0), None],
    );
    assert!(!value_may_throw(&ir, ValueUse::Reference(V0)));
    assert!(value_may_throw(&ir, ValueUse::Definition(V0)));
    assert!(value_may_throw(&ir, ValueUse::Reference(V1)));
    assert_eq!(
        values(&ir, ValueUse::Reference(V1))
            .operations()
            .collect::<Vec<_>>(),
        vec![V1, V0]
    );
    assert!(!statement_may_throw(
        &ir,
        &Statement::Assign {
            local: L1,
            value: ValueUse::Reference(V0)
        }
    ));
    assert!(statement_may_throw(
        &ir,
        &Statement::Assign {
            local: L0,
            value: ValueUse::Definition(V0)
        }
    ));
}

#[test]
fn assignment_use_survives_storage_and_operand_renaming() {
    let mut ir = fixture(vec![call(), call()], vec![Some(L0), Some(L1)]);
    let mut copied = Statement::Assign {
        local: L1,
        value: ValueUse::Reference(V0),
    };
    let mut definition = Statement::Assign {
        local: L0,
        value: ValueUse::Definition(V0),
    };
    // Coalescing can make a read and its destination share storage. Retargeting
    // a definition can do the reverse. Neither operation changes evaluation.
    if let Statement::Assign { local, .. } = &mut copied {
        *local = L0;
    }
    if let Statement::Assign { local, .. } = &mut definition {
        *local = L1;
    }
    assert!(!statement_may_throw(&ir, &copied));
    assert!(statement_may_throw(&ir, &definition));
    copied.map_operands(|_| V1);
    definition.map_operands(|_| V1);
    ir.values[V1].local = Some(L0);
    assert!(!statement_may_throw(&ir, &copied));
    assert!(statement_may_throw(&ir, &definition));
    assert_eq!(
        statement_values(&ir, &copied).collect::<Vec<_>>(),
        [ValueUse::Reference(V1)]
    );
    assert_eq!(
        statement_values(&ir, &definition).collect::<Vec<_>>(),
        [ValueUse::Definition(V1)]
    );
}

#[test]
fn phi_inputs_are_edge_metadata_and_do_not_reevaluate_incoming_calls() {
    let ir = fixture(
        vec![
            call(),
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
                        value: V1,
                    },
                ],
                slot: Some(0),
            },
        ],
        vec![None, None],
    );
    assert!(!value_may_throw(&ir, ValueUse::Definition(V1)));
    assert_eq!(
        values(&ir, ValueUse::Definition(V1))
            .operations()
            .collect::<Vec<_>>(),
        vec![V1]
    );
}

#[test]
fn parameter_definitions_are_reads_and_shared_dependencies_are_bounded() {
    let ir = fixture(
        vec![
            ValueKind::Parameter {
                slot: 0,
                index: Some(0),
            },
            ValueKind::Alias(V2),
            ValueKind::Alias(V1),
        ],
        vec![Some(L0), None, None],
    );
    assert_eq!(ValueUse::Definition(V0).local(&ir), Some(L0));
    assert_eq!(
        values(&ir, ValueUse::Definition(V0)).operations().count(),
        0
    );
    assert_eq!(
        values(&ir, ValueUse::Reference(V1))
            .operations()
            .collect::<Vec<_>>(),
        vec![V1, V2]
    );
}
