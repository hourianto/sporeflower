use super::*;
use crate::cfg::HandlerEdge;
use crate::classfile::{JvmType, LocalVariable};
use crate::index::test_ids::*;
use crate::index::{BlockId, LocalId, ValueId};
use crate::ir::ValueUse;
use crate::ir::ValueUse::{Definition, Reference};
use crate::ir::{
    CompareOp, Condition, InvokeKind, IrBlock, Literal, Local, Member, Value, ValueKind,
};

fn block(pc: u32, statements: Vec<Statement>, terminator: Terminator) -> IrBlock {
    IrBlock {
        caught_exception: None,
        pc,
        statements,
        terminator,
        handlers: vec![],
    }
}

fn assign(local: usize, value: ValueUse) -> Statement {
    Statement::Assign {
        local: LocalId::from_index(local),
        value,
    }
}

fn fixture() -> MethodIr {
    let values = [
        (
            ValueKind::Parameter {
                slot: 0,
                index: Some(0),
            },
            0,
            0,
        ),
        (
            ValueKind::Call {
                kind: InvokeKind::Static,
                method: Member {
                    owner: "Probe".into(),
                    name: "select".into(),
                    descriptor: "(I)I".into(),
                },
                receiver: None,
                args: vec![V0],
            },
            2,
            0,
        ),
        (ValueKind::Literal(Literal::Int(0)), 5, 0),
        (ValueKind::Literal(Literal::Int(1)), 30, 2),
        (
            ValueKind::Phi {
                inputs: vec![],
                slot: Some(2),
            },
            40,
            3,
        ),
        (ValueKind::Literal(Literal::Int(-1)), 50, 4),
        (
            ValueKind::InlineAssign {
                local: L2,
                value: V3,
            },
            6,
            0,
        ),
        (
            ValueKind::Select {
                condition: Condition {
                    op: CompareOp::Eq,
                    left: V0,
                    right: V5,
                },
                then_value: V6,
                else_value: V1,
            },
            7,
            0,
        ),
    ];
    {
        let mut ir = MethodIr {
            entry: B0,
            values: values
                .into_iter()
                .map(|(kind, pc, block)| Value {
                    local: None,
                    kind,
                    pc,
                    block: BlockId::from_index(block),
                    ty: JvmType::Int,
                })
                .collect(),
            blocks: vec![
                block(
                    0,
                    vec![assign(1, Definition(V1))],
                    Terminator::Switch {
                        value: V1,
                        cases: vec![(17, B1), (32, B2)],
                        default: B4,
                    },
                ),
                block(0, vec![assign(2, Reference(V2))], Terminator::Goto(B3)),
                block(30, vec![assign(2, Reference(V3))], Terminator::Goto(B3)),
                block(40, vec![], Terminator::Return(Some(V4))),
                block(50, vec![], Terminator::Return(Some(V5))),
            ]
            .into(),
            locals: (0..3)
                .map(|id| Local {
                    name_hint: None,
                    name: format!("v{id}"),
                    ty: JvmType::Int,
                    parameter: (id == 0).then_some(0),
                    is_this: false,
                    needs_default: false,
                })
                .collect(),
            parameters: vec![V0],
            return_type: JvmType::Int,
            initialization_sites: vec![],
        };
        for (entity, metadata) in ir.values.iter_mut().zip(vec![
            Some(0),
            Some(1),
            None,
            None,
            Some(2),
            None,
            None,
            None,
        ]) {
            entity.local = metadata.map(LocalId::from_index);
        }
        for (entity, metadata) in ir.locals.iter_mut().zip(vec![
            None,
            None,
            Some(LocalVariable {
                start_pc: 5,
                length: 46,
                name: "flag".into(),
                descriptor: "I".into(),
                index: 2,
            }),
        ]) {
            entity.name_hint = metadata;
        }
        ir
    }
}

fn lower(ir: &mut MethodIr) -> IdVec<BlockId, Vec<u32>> {
    let mut raw = crate::ir::lift::RawIr::new(ir.clone());
    for (block, body) in ir.blocks.iter_enumerated() {
        raw.set_statements(
            block,
            body.statements
                .iter()
                .map(|statement| {
                    let pc = match statement {
                        Statement::Assign { value, .. } => ir.values[value.value()].pc,
                        _ => body.pc,
                    };
                    (statement.clone(), pc)
                })
                .collect(),
        );
    }
    hoist(&mut raw);
    raw.assert_statement_origins();
    let pcs = raw
        .ir
        .blocks
        .ids()
        .map(|block| raw.statement_pcs(block).to_vec())
        .collect();
    *ir = raw.ir;
    pcs
}

#[test]
fn shared_case_default_moves_after_selector_statements_with_its_original_identity() {
    let mut ir = fixture();
    let caught = ir
        .blocks
        .iter()
        .map(|block| block.caught_exception)
        .collect::<Vec<_>>();
    let hints = format!(
        "{:?}",
        ir.locals
            .iter()
            .map(|local| &local.name_hint)
            .collect::<Vec<_>>()
    );
    let value = format!("{:?}", ir.values[V2]);
    let pcs = lower(&mut ir);
    assert!(ir.blocks[B1].statements.is_empty());
    assert!(matches!(
        ir.blocks[B0].statements.as_slice(),
        [
            Statement::Assign {
                local: L1,
                value: Definition(V1)
            },
            Statement::Assign {
                local: L2,
                value: Reference(V2)
            }
        ]
    ));
    assert!(matches!(
        ir.blocks[B2].statements.as_slice(),
        [Statement::Assign {
            local: L2,
            value: Reference(V3)
        }]
    ));
    assert_eq!(pcs[B0], vec![2, 5]);
    assert_eq!(
        ir.blocks
            .iter()
            .map(|block| block.caught_exception)
            .collect::<Vec<_>>(),
        caught
    );
    assert_eq!(
        format!(
            "{:?}",
            ir.locals
                .iter()
                .map(|local| &local.name_hint)
                .collect::<Vec<_>>()
        ),
        hints
    );
    assert_eq!(format!("{:?}", ir.values[V2]), value);
    assert!(matches!(ir.blocks[B1].terminator, Terminator::Goto(B3)));
}

#[test]
fn normal_old_value_and_loop_backedge_reads_prevent_hoisting() {
    for backedge in [false, true] {
        let mut ir = fixture();
        ir.blocks[B4].statements.push(assign(1, Reference(V4)));
        if backedge {
            ir.blocks[B4].terminator = Terminator::Goto(B0);
        }
        lower(&mut ir);
        assert_eq!(ir.blocks[B0].statements.len(), 1);
        assert_eq!(ir.blocks[B1].statements.len(), 1);
    }
}

#[test]
fn acyclic_choices_keep_literal_results_with_their_expression_owner() {
    let mut ir = fixture();
    ir.blocks[B0].terminator = Terminator::Branch {
        condition: Condition {
            op: CompareOp::Eq,
            left: V0,
            right: V5,
        },
        then_target: B1,
        else_target: B2,
    };
    lower(&mut ir);
    assert_eq!(ir.blocks[B0].statements.len(), 1);
    assert_eq!(ir.blocks[B1].statements.len(), 1);
    assert_eq!(ir.blocks[B2].statements.len(), 1);
}

#[test]
fn only_transported_initializers_need_definition_owned_storage() {
    for transported in [false, true] {
        let mut ir = fixture();
        ir.blocks[B0].terminator = Terminator::Goto(B1);
        ir.blocks[B1].terminator = Terminator::Branch {
            condition: Condition {
                op: CompareOp::Eq,
                left: V0,
                right: V5,
            },
            then_target: B2,
            else_target: B4,
        };
        ir.blocks[B2].terminator = Terminator::Goto(B1);
        ir.values[V4].block = B1;
        let entry = if transported {
            ir.blocks.push(block(4, vec![], Terminator::Goto(B1)));
            ir.blocks[B0].terminator = Terminator::Goto(B5);
            B5
        } else {
            B0
        };
        ir.values[V4].kind = ValueKind::Phi {
            inputs: vec![
                crate::ir::PhiInput {
                    block: entry,
                    throw_pc: None,
                    value: V2,
                },
                crate::ir::PhiInput {
                    block: B2,
                    throw_pc: None,
                    value: V3,
                },
            ],
            slot: Some(2),
        };
        let mut raw = crate::ir::lift::RawIr::new(ir);
        for value in [V2, V3] {
            raw.local_accesses.push(crate::ir::lift::LocalAccess {
                value,
                replaced: None,
                slot: 2,
                pc: raw.ir.values[value].pc,
                next_pc: raw.ir.values[value].pc + 1,
                store: true,
            });
        }
        let mut used: IdVec<ValueId, _> = vec![false; raw.ir.values.len()].into();
        used[V4] = true;
        let uses = vec![0; raw.ir.values.len()].into();
        let captures = definitions(&raw, &used, &uses);
        assert_eq!(captures.len(), if transported { 2 } else { 0 });
    }
}

#[test]
fn literal_stored_before_a_branch_keeps_its_definition_instead_of_an_else_copy() {
    let mut ir = fixture();
    ir.blocks[B0].terminator = Terminator::Branch {
        condition: Condition {
            op: CompareOp::Eq,
            left: V0,
            right: V5,
        },
        then_target: B2,
        else_target: B3,
    };
    ir.values[V4].kind = ValueKind::Phi {
        inputs: vec![
            crate::ir::PhiInput {
                block: B0,
                throw_pc: None,
                value: V2,
            },
            crate::ir::PhiInput {
                block: B2,
                throw_pc: None,
                value: V3,
            },
        ],
        slot: Some(2),
    };
    let mut raw = crate::ir::lift::RawIr::new(ir);
    raw.local_accesses.push(crate::ir::lift::LocalAccess {
        value: V2,
        replaced: None,
        slot: 2,
        pc: raw.ir.values[V2].pc,
        next_pc: raw.ir.values[V2].pc + 1,
        store: true,
    });
    let mut used: IdVec<ValueId, _> = vec![false; raw.ir.values.len()].into();
    used[V4] = true;
    let mut uses: IdVec<ValueId, _> = vec![0; raw.ir.values.len()].into();
    uses[V4] = 1;
    assert!(definitions(&raw, &used, &uses).is_empty());
    // One result use still needs statement storage when the alternate arm has
    // an independent write that cannot belong to its conditional operand.
    raw.events[B2].push(crate::ir::lift::Event {
        pc: 31,
        kind: crate::ir::lift::EventKind::Action(Statement::PutField {
            pc: Some(31),
            field: Member {
                owner: "Probe".into(),
                name: "trace".into(),
                descriptor: "I".into(),
            },
            receiver: None,
            value: V3,
        }),
    });
    assert_eq!(
        definitions(&raw, &used, &uses),
        BTreeMap::from([(V2, JvmType::Int)])
    );
    raw.events[B2].clear();
    uses[V4] = 2;
    assert_eq!(
        definitions(&raw, &used, &uses),
        BTreeMap::from([(V2, JvmType::Int)])
    );

    // A literal carried on the operand stack has no original local binding to
    // retain. Do not invent one merely because a branch can reach its join.
    raw.local_accesses.clear();
    assert!(definitions(&raw, &used, &uses).is_empty());
}

#[test]
fn selector_reads_and_conditional_inline_writes_prevent_hoisting() {
    for selector in [4, 6, 7] {
        let mut ir = fixture();
        if let Terminator::Switch { value, .. } = &mut ir.blocks[B0].terminator {
            *value = ValueId::from_index(selector);
        }
        lower(&mut ir);
        assert_eq!(ir.blocks[B0].statements.len(), 1);
        assert_eq!(ir.blocks[B1].statements.len(), 1);
    }
}

#[test]
fn throwing_selector_preserves_the_old_value_observed_by_its_handler() {
    let mut ir = fixture();
    ir.blocks[B0].statements.clear();
    ir.values[V1].local = None; // The call now evaluates in the terminator.
    ir.blocks
        .push(block(60, vec![], Terminator::Return(Some(V4))));

    ir.blocks[B0].handlers.push(HandlerEdge {
        target: B5,
        catch_type: None,
        table_index: 0,
    });
    lower(&mut ir);
    assert!(ir.blocks[B0].statements.is_empty());
    assert_eq!(ir.blocks[B1].statements.len(), 1);
    assert_eq!(ir.blocks[B0].handlers[0].target, B5);
}

#[test]
fn completed_selector_statement_keeps_its_earlier_exception_observation() {
    let mut ir = fixture();
    ir.blocks
        .push(block(60, vec![], Terminator::Return(Some(V4))));

    ir.blocks[B0].handlers.push(HandlerEdge {
        target: B5,
        catch_type: None,
        table_index: 0,
    });
    lower(&mut ir);
    // Its handler sees the old flag if select() throws. The new literal
    // executes only after select() has returned, so moving it is safe.
    assert!(ir.blocks[B1].statements.is_empty());
    assert!(matches!(
        ir.blocks[B0].statements.as_slice(),
        [
            Statement::Assign {
                local: L1,
                value: Definition(V1)
            },
            Statement::Assign {
                local: L2,
                value: Reference(V2)
            }
        ]
    ));
}

#[test]
fn shared_normal_entries_use_their_owner_but_handler_entries_keep_the_assignment() {
    for exceptional in [false, true] {
        let mut ir = fixture();
        if exceptional {
            ir.blocks[B4].handlers.push(HandlerEdge {
                target: B1,
                catch_type: None,
                table_index: 0,
            });
        } else {
            ir.blocks[B4].terminator = Terminator::Goto(B1);
        }
        lower(&mut ir);
        assert_eq!(ir.blocks[B1].statements.len(), usize::from(exceptional));
        assert!(
            ir.blocks[B0].statements.iter().any(|statement| matches!(
                statement,
                Statement::Assign {
                    value: Reference(V2),
                    ..
                }
            )) != exceptional
        );
    }
}

#[test]
fn unshared_and_unused_defaults_do_not_move() {
    for unused in [false, true] {
        let mut ir = fixture();
        if unused {
            ir.blocks[B3].terminator = Terminator::Return(Some(V5));
        } else {
            ir.blocks[B2].terminator = Terminator::Return(Some(V3));
        }
        lower(&mut ir);
        assert_eq!(ir.blocks[B0].statements.len(), 1);
        assert_eq!(ir.blocks[B1].statements.len(), 1);
    }
}

#[test]
fn an_intervening_write_must_not_outlive_the_default_it_precedes() {
    let mut ir = fixture();
    ir.blocks[B4].statements.push(assign(2, Reference(V3)));
    ir.blocks[B4].terminator = Terminator::Goto(B1);
    lower(&mut ir);
    assert_eq!(ir.blocks[B1].statements.len(), 1);
    assert!(!ir.blocks[B0].statements.iter().any(|statement| {
        matches!(
            statement,
            Statement::Assign {
                value: Reference(V2),
                ..
            }
        )
    }));
}

#[test]
fn defaults_consumed_inside_a_loop_are_initialized_on_every_iteration() {
    let mut ir = fixture();
    ir.blocks.push(block(60, vec![], Terminator::Goto(B0)));
    ir.entry = B5;
    ir.blocks[B3].statements.push(assign(1, Reference(V4)));
    ir.blocks[B3].terminator = Terminator::Goto(B0);
    lower(&mut ir);
    assert!(ir.blocks[B1].statements.is_empty());
    assert_eq!(ir.blocks[B0].statements.len(), 2);
    assert!(ir.blocks[B5].statements.is_empty());
}

#[test]
fn a_feedback_choice_updates_existing_storage_instead_of_defaulting_a_new_result() {
    let mut ir = fixture();
    ir.blocks[B2].statements = vec![assign(2, Reference(V1))];
    ir.blocks[B3].statements.push(assign(1, Reference(V4)));
    ir.blocks[B3].terminator = Terminator::Goto(B0);
    lower(&mut ir);
    assert_eq!(ir.blocks[B0].statements.len(), 1);
    assert_eq!(ir.blocks[B1].statements.len(), 1);
}

#[test]
fn null_is_eligible_but_resolvable_reference_constants_are_not() {
    for (literal, expected) in [
        (Literal::Null, true),
        (Literal::String(vec![65]), false),
        (Literal::Class(JvmType::Object("Probe".into())), false),
    ] {
        let mut ir = fixture();
        let ty = JvmType::Object("java/lang/Object".into());
        ir.locals[L2].ty = ty.clone();
        ir.return_type = ty.clone();
        for value in [2, 3, 4] {
            ir.values[ValueId::from_index(value)].ty = ty.clone();
        }
        ir.values[V2].kind = ValueKind::Literal(literal.clone());
        ir.values[V3].kind = ValueKind::Literal(literal);
        lower(&mut ir);
        assert_eq!(ir.blocks[B1].statements.is_empty(), expected);
    }
}
