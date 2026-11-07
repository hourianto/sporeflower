use super::*;
use crate::classfile::JvmType;
use crate::index::test_ids::*;
use crate::index::{LocalId, ValueId};
use crate::ir::ValueUse;
use crate::ir::scheduled_liveness::LocalLiveness;
use crate::ir::{CompareOp, Condition, Literal, Local, Value};
use crate::ir::{InvokeKind, IrBlock, Member, Terminator};

#[test]
fn local_accesses_include_alternative_captures_but_stop_at_materialized_reads() {
    let kinds = vec![
        ValueKind::Literal(Literal::Int(7)),
        ValueKind::InlineAssign {
            local: L1,
            value: V0,
        },
        ValueKind::Literal(Literal::Int(9)),
        ValueKind::InlineAssign {
            local: L3,
            value: V2,
        },
        ValueKind::Parameter {
            slot: 4,
            index: Some(0),
        },
        ValueKind::Select {
            condition: Condition {
                op: CompareOp::Eq,
                left: V4,
                right: V0,
            },
            then_value: V1,
            else_value: V3,
        },
    ];
    let mut ir = {
        let mut ir = MethodIr {
            entry: B0,
            blocks: vec![].into(),
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
            locals: (0..6)
                .map(|local| Local {
                    name_hint: None,
                    name: format!("v{local}"),
                    ty: JvmType::Int,
                    parameter: None,
                    is_this: false,
                    needs_default: false,
                })
                .collect(),
            parameters: vec![V4],
            return_type: JvmType::Int,
            initialization_sites: vec![],
        };
        for (entity, metadata) in
            ir.values
                .iter_mut()
                .zip(vec![Some(0), None, Some(2), None, Some(4), None])
        {
            entity.local = metadata.map(LocalId::from_index);
        }
        ir
    };

    // Both arms can write, but neither assignment is definite after the select.
    // Capture RHSs evaluate their definitions rather than reading locals 0/2.
    assert_eq!(
        value_accesses(&ir, ValueUse::Reference(V5)),
        BTreeSet::from([0, 1, 3, 4].map(LocalId::from_index))
    );
    assert_eq!(
        value_assignments(&ir, ValueUse::Reference(V5)),
        BTreeSet::from([1, 3].map(LocalId::from_index))
    );
    let (mut reads, mut defined) = (BTreeSet::new(), BTreeSet::new());
    expression_effects(&ir, ValueUse::Reference(V5), &mut reads, &mut defined);
    assert_eq!(reads, BTreeSet::from([0, 4].map(LocalId::from_index)));
    assert!(defined.is_empty());

    ir.values[V5].local = Some(L5);
    let copy = Statement::Assign {
        local: L2,
        value: ValueUse::Reference(V5),
    };
    assert_eq!(
        statement_accesses(&ir, &copy),
        BTreeSet::from([2, 5].map(LocalId::from_index))
    );
    assert_eq!(
        statement_assignments(&ir, &copy),
        BTreeSet::from([2].map(LocalId::from_index))
    );
    assert_eq!(
        value_assignments(&ir, ValueUse::Reference(V5)),
        BTreeSet::new()
    );
    assert_eq!(
        value_assignments(&ir, ValueUse::Definition(V5)),
        BTreeSet::from([1, 3].map(LocalId::from_index))
    );

    let one = ir.values.next_id();
    ir.values.push(Value {
        local: None,
        kind: ValueKind::Literal(Literal::Int(1)),
        ty: JvmType::Int,
        pc: 1,
        block: B0,
    });

    let next = ir.values.next_id();
    ir.values.push(Value {
        local: None,
        kind: ValueKind::Binary {
            op: crate::ir::BinaryOp::Add,
            left: V4,
            right: one,
        },
        ty: JvmType::Int,
        pc: 1,
        block: B0,
    });
    ir.values[next].local = Some(L4);
    let old = ir.values.next_id();
    ir.values.push(Value {
        local: None,
        kind: ValueKind::LocalUpdate {
            local: L4,
            value: next,
            increment: true,
            postfix: true,
        },
        ty: JvmType::Int,
        pc: 1,
        block: B0,
    });

    let effects = local_effects::expression(&ir, ValueUse::Reference(old));
    let mut writes = Vec::new();
    effects.assignments(&mut |local, value| writes.push((local, value)));
    assert_eq!(
        writes,
        [(L4, ValueUse::Definition(next))],
        "the committed RHS is distinct from the yielded old value"
    );
    assert_eq!(
        value_reads(&ir, ValueUse::Reference(old)),
        BTreeSet::from([4].map(LocalId::from_index))
    );
    assert_eq!(
        value_assignments(&ir, ValueUse::Reference(old)),
        BTreeSet::from([4].map(LocalId::from_index))
    );
    assert!(!crate::ir::scheduled::value_may_throw(
        &ir,
        ValueUse::Reference(old)
    ));
}

#[test]
fn handler_liveness_observes_writes_at_their_actual_throw_boundary() {
    let mut ir = {
        let mut ir = MethodIr {
            entry: B0,
            blocks: vec![
                IrBlock {
                    caught_exception: None,
                    pc: 0,
                    statements: vec![
                        Statement::Eval(V1),
                        Statement::Assign {
                            local: L0,
                            value: ValueUse::Definition(V2),
                        },
                    ],
                    terminator: Terminator::Return(Some(V2)),
                    handlers: vec![crate::cfg::HandlerEdge {
                        target: B1,
                        catch_type: None,
                        table_index: 0,
                    }],
                },
                IrBlock {
                    caught_exception: None,
                    pc: 10,
                    statements: vec![],
                    terminator: Terminator::Return(Some(V0)),
                    handlers: vec![],
                },
            ]
            .into(),
            values: vec![
                Value {
                    local: None,
                    kind: ValueKind::Parameter {
                        slot: 0,
                        index: Some(0),
                    },
                    ty: JvmType::Int,
                    pc: 0,
                    block: B0,
                },
                Value {
                    local: None,
                    kind: ValueKind::Call {
                        kind: InvokeKind::Static,
                        method: Member {
                            owner: "Input".into(),
                            name: "read".into(),
                            descriptor: "()I".into(),
                        },
                        receiver: None,
                        args: vec![],
                    },
                    ty: JvmType::Int,
                    pc: 0,
                    block: B0,
                },
                Value {
                    local: None,
                    kind: ValueKind::Literal(Literal::Int(7)),
                    ty: JvmType::Int,
                    pc: 3,
                    block: B0,
                },
                Value {
                    local: None,
                    kind: ValueKind::CaughtException { handler: B1 },
                    ty: JvmType::Object("java/lang/Throwable".into()),
                    pc: 10,
                    block: B1,
                },
            ]
            .into(),
            locals: vec![
                Local {
                    name_hint: None,
                    name: "input".into(),
                    ty: JvmType::Int,
                    parameter: Some(0),
                    is_this: false,
                    needs_default: false,
                },
                Local {
                    name_hint: None,
                    name: "e".into(),
                    ty: JvmType::Object("java/lang/Throwable".into()),
                    parameter: None,
                    is_this: false,
                    needs_default: false,
                },
            ]
            .into(),
            parameters: vec![V0],
            return_type: JvmType::Int,
            initialization_sites: vec![],
        };
        for (entity, metadata) in ir
            .values
            .iter_mut()
            .zip(vec![Some(0), None, Some(0), Some(1)])
        {
            entity.local = metadata.map(LocalId::from_index);
        }
        for (entity, metadata) in ir.blocks.iter_mut().zip(vec![None, Some(3)]) {
            entity.caught_exception = metadata.map(ValueId::from_index);
        }
        ir
    };
    // A throw before the assignment exposes the incoming value to the handler.
    assert_eq!(
        live_at_entry(&ir).unwrap(),
        BTreeSet::from([0].map(LocalId::from_index))
    );
    ir.blocks[B0].statements.swap(0, 1);
    assert_eq!(live_at_entry(&ir).unwrap(), BTreeSet::new());

    // A block with both kinds of predecessor receives its exception binding
    // only on the exceptional edge. A normal edge still needs that local.
    ir.return_type = JvmType::Object("java/lang/Throwable".into());
    ir.blocks[B0].terminator = Terminator::Goto(B1);
    ir.blocks[B1].terminator = Terminator::Return(Some(V3));
    let live = LocalLiveness::analyze(&ir).unwrap();
    assert_eq!(live.at(B0), BTreeSet::from([1].map(LocalId::from_index)));
    assert_eq!(live.at(B1), BTreeSet::from([1].map(LocalId::from_index)));
    assert_eq!(live.at_handler(&ir, B1), BTreeSet::new());
}
