use super::*;
use crate::cfg::HandlerEdge;
use crate::classfile::{JvmType, LocalVariable};
use crate::index::test_ids::*;
use crate::index::{LocalId, ValueId};
use crate::ir::ValueUse;
use crate::ir::ValueUse::{Definition, Reference};
use crate::ir::{
    CompareOp, Condition, InvokeKind, IrBlock, Literal, Local, Member, Terminator, Value,
};

fn block(statements: Vec<Statement>, terminator: Terminator) -> IrBlock {
    IrBlock {
        caught_exception: None,
        pc: 0,
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

fn fixture(blocks: Vec<IrBlock>) -> MethodIr {
    // V0/V1 denote distinct physical storage, V2 is an observable definition
    // of local 0, and V3 is an inline write to that same physical storage.
    let kinds = vec![
        ValueKind::Literal(Literal::Int(7)),
        ValueKind::Literal(Literal::Int(0)),
        ValueKind::Call {
            kind: InvokeKind::Static,
            method: Member {
                owner: "Probe".into(),
                name: "next".into(),
                descriptor: "()I".into(),
            },
            receiver: None,
            args: vec![],
        },
        ValueKind::InlineAssign {
            local: L0,
            value: V4,
        },
        ValueKind::Literal(Literal::Int(9)),
    ];
    {
        let mut ir = MethodIr {
            entry: B0,
            blocks: blocks.into(),
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
            locals: (0..2)
                .map(|index| Local {
                    name_hint: None,
                    name: format!("v{index}"),
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
        for (entity, metadata) in
            ir.values
                .iter_mut()
                .zip(vec![Some(0), Some(1), Some(0), None, None])
        {
            entity.local = metadata.map(LocalId::from_index);
        }
        ir
    }
}

fn lower(ir: &mut MethodIr) -> usize {
    let mut raw = RawIr::new(ir.clone());
    let result = run(&mut raw);
    raw.assert_statement_origins();
    *ir = raw.ir;
    result
}

fn protected(ir: &mut MethodIr) {
    ir.blocks[B0].handlers.push(HandlerEdge {
        target: B1,
        catch_type: None,
        table_index: 0,
    });
}

fn sibling_fixture(intervening_write: bool) -> MethodIr {
    let mut statements = vec![assign(0, Definition(V2)), assign(1, Reference(V2))];
    if intervening_write {
        statements.push(Statement::Eval(V3));
    }
    statements.push(assign(2, Reference(V2)));
    if !intervening_write {
        statements.push(Statement::Eval(V3));
    }
    statements.push(Statement::Eval(V7));
    statements.push(assign(2, Reference(V1)));
    let mut ir = fixture(vec![block(statements, Terminator::Return(Some(V6)))]);
    ir.locals.push(Local {
        name_hint: None,
        name: "v2".into(),
        ty: JvmType::Int,
        parameter: None,
        is_this: false,
        needs_default: false,
    });
    ir.values.extend([
        Value {
            local: None,
            kind: ValueKind::Phi {
                inputs: vec![],
                slot: None,
            },
            ty: JvmType::Int,
            pc: 0,
            block: B0,
        },
        Value {
            local: None,
            kind: ValueKind::Binary {
                op: crate::ir::BinaryOp::Add,
                left: V1,
                right: V5,
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
                    owner: "Probe".into(),
                    name: "use".into(),
                    descriptor: "(I)V".into(),
                },
                receiver: None,
                args: vec![V5],
            },
            ty: JvmType::Void,
            pc: 0,
            block: B0,
        },
    ]);
    ir.values[V5].local = Some(L2);
    ir
}

#[test]
fn source_slot_affinity_wins_over_a_transient_stack_copy() {
    // L1 is the old cursor, L2 its next version, and L0 an index capture.
    // L0 can share storage with L1, but then the L1/L2 merge is impossible.
    let mut ir = fixture(vec![block(
        vec![
            assign(1, Reference(V4)),
            assign(0, Reference(V1)),
            assign(2, Definition(V5)),
            Statement::Eval(V6),
            assign(1, Reference(V2)),
        ],
        Terminator::Return(Some(V1)),
    )]);
    ir.locals.push(ir.locals[L1].clone());
    for (value, slot) in [(0, None), (1, Some(8)), (2, Some(8))] {
        let local = LocalId::from_index(value);
        let value = ValueId::from_index(value);
        ir.values[value].kind = ValueKind::Phi {
            inputs: vec![],
            slot,
        };
        ir.values[value].local = Some(local);
    }
    for (left, right, local) in [(1, 4, Some(2)), (0, 2, None)] {
        ir.values.push(Value {
            local: None,
            kind: ValueKind::Binary {
                op: crate::ir::BinaryOp::Add,
                left: ValueId::from_index(left),
                right: ValueId::from_index(right),
            },
            ty: JvmType::Int,
            pc: 0,
            block: B0,
        });
        ir.values.iter_mut().last().unwrap().local = local.map(LocalId::from_index);
    }
    assert_eq!(lower(&mut ir), 1);
    assert_eq!(ir.values[V1].local, ir.values[V2].local);
    assert_ne!(ir.values[V0].local, ir.values[V1].local);
}

#[test]
fn a_reused_jvm_slot_does_not_override_interference() {
    let mut ir = fixture(vec![block(
        vec![
            assign(0, Definition(V2)),
            assign(1, Reference(V4)),
            assign(1, Reference(V0)),
        ],
        Terminator::Return(Some(V0)),
    )]);
    for value in ir.values.iter_mut().take(2) {
        value.kind = ValueKind::Phi {
            inputs: vec![],
            slot: Some(8),
        };
    }
    assert_eq!(lower(&mut ir), 0);
    assert_ne!(ir.values[V0].local, ir.values[V1].local);
}

#[test]
fn sibling_snapshots_share_storage_after_their_source_is_overwritten() {
    let mut ir = sibling_fixture(false);
    assert!(lower(&mut ir) > 0);
    assert_eq!(ir.values[V1].local, ir.values[V5].local);
    assert_ne!(ir.values[V0].local, ir.values[V1].local);
}

#[test]
fn a_write_between_snapshots_prevents_content_based_coalescing() {
    let mut ir = sibling_fixture(true);
    lower(&mut ir);
    assert_ne!(ir.values[V1].local, ir.values[V5].local);
}

#[test]
fn equal_constant_stores_do_not_conflict_but_different_contents_do() {
    for (constant, merge) in [(7, true), (9, false)] {
        let mut ir = fixture(vec![block(
            vec![
                assign(0, Definition(V0)),
                assign(1, Reference(V4)),
                assign(1, Reference(V0)),
            ],
            Terminator::Return(Some(V1)),
        )]);
        ir.values[V4].kind = ValueKind::Literal(Literal::Int(constant));
        assert_eq!(lower(&mut ir) != 0, merge);
        assert_eq!(ir.values[V0].local == ir.values[V1].local, merge);
    }
}

#[test]
fn loop_contents_must_agree_on_reentry_not_just_the_initial_path() {
    let mut ir = fixture(vec![
        block(
            vec![assign(0, Definition(V0)), assign(1, Reference(V4))],
            Terminator::Goto(B1),
        ),
        block(
            vec![assign(1, Reference(V4))],
            Terminator::Branch {
                condition: Condition {
                    op: CompareOp::Eq,
                    left: V0,
                    right: V4,
                },
                then_target: B2,
                else_target: B3,
            },
        ),
        block(vec![Statement::Eval(V3)], Terminator::Goto(B1)),
        block(vec![assign(1, Reference(V0))], Terminator::Return(Some(V1))),
    ]);
    ir.values[V4].kind = ValueKind::Literal(Literal::Int(7));
    assert_eq!(lower(&mut ir), 0);
}

#[test]
fn exception_entry_does_not_inherit_old_normal_content_facts() {
    let mut ir = fixture(vec![
        block(
            vec![
                assign(0, Definition(V0)),
                assign(1, Reference(V0)),
                Statement::Eval(V3),
                Statement::Eval(V2),
            ],
            Terminator::Return(Some(V0)),
        ),
        block(vec![assign(1, Reference(V4))], Terminator::Return(Some(V0))),
    ]);
    protected(&mut ir);
    ir.values[V4].kind = ValueKind::Literal(Literal::Int(7));
    assert_eq!(lower(&mut ir), 0);
}

#[test]
fn expression_internal_writes_keep_full_interference() {
    let mut ir = fixture(vec![block(
        vec![
            assign(0, Definition(V0)),
            assign(1, Reference(V4)),
            assign(1, Reference(V3)),
            assign(1, Reference(V0)),
        ],
        Terminator::Return(Some(V0)),
    )]);
    ir.values[V4].kind = ValueKind::Literal(Literal::Int(7));
    assert_eq!(lower(&mut ir), 0);
}

#[test]
fn coalescing_removes_copies_and_preserves_producer_evaluation() {
    let ir = fixture(vec![block(
        vec![assign(0, Definition(V2)), assign(1, Reference(V2))],
        Terminator::Return(Some(V1)),
    )]);
    let mut raw = RawIr::new(ir);
    raw.set_statements(
        B0,
        vec![
            (assign(0, Definition(V2)), 11),
            (assign(1, Reference(V2)), 19),
        ],
    );
    assert_eq!(run(&mut raw), 1);
    assert_eq!(raw.statement_pcs(B0), &[11]);
    raw.assert_statement_origins();
    let ir = raw.ir;
    assert_eq!(ir.values[V1].local, ir.values[V2].local);
    assert_eq!(ir.blocks[B0].statements.len(), 1);
    assert!(matches!(
        ir.blocks[B0].statements[0],
        Statement::Assign {
            value: Definition(V2),
            ..
        }
    ));
    // The call remains one immutable value and one evaluated definition.
    assert!(matches!(ir.values[V2].kind, ValueKind::Call { .. }));
}

#[test]
fn exception_reads_observe_old_storage_until_rhs_commits() {
    let mut ir = fixture(vec![
        block(
            vec![
                assign(1, Definition(V1)),
                assign(0, Definition(V2)),
                assign(1, Reference(V2)),
            ],
            Terminator::Return(Some(V0)),
        ),
        block(vec![], Terminator::Return(Some(V1))),
    ]);
    protected(&mut ir);
    assert_eq!(lower(&mut ir), 1);
    assert_eq!(ir.values[V0].local, ir.values[V1].local);
    // A throw between the definition and snapshot must keep both storages.
    let mut ir = fixture(vec![
        block(
            vec![
                assign(1, Definition(V1)),
                assign(0, Definition(V2)),
                Statement::Eval(V2),
                assign(1, Reference(V2)),
            ],
            Terminator::Return(Some(V0)),
        ),
        block(vec![], Terminator::Return(Some(V1))),
    ]);
    protected(&mut ir);
    assert_eq!(lower(&mut ir), 0);
}

#[test]
fn inline_capture_before_a_throw_interferes_with_saved_handler_storage() {
    let mut ir = fixture(vec![
        block(
            vec![
                assign(0, Definition(V0)),
                assign(1, Reference(V0)),
                Statement::Eval(V3),
                Statement::Eval(V2),
            ],
            Terminator::Return(Some(V0)),
        ),
        block(vec![], Terminator::Return(Some(V1))),
    ]);
    protected(&mut ir);
    assert_eq!(lower(&mut ir), 0);
}

#[test]
fn shadowed_handler_reads_do_not_prevent_storage_coalescing() {
    let mut ir = fixture(vec![
        block(
            vec![
                assign(0, Definition(V0)),
                assign(1, Reference(V0)),
                Statement::Eval(V3),
                Statement::Eval(V2),
            ],
            Terminator::Return(Some(V0)),
        ),
        block(vec![], Terminator::Return(Some(V4))),
        block(vec![], Terminator::Return(Some(V1))),
    ]);
    for target in [B1, B2] {
        ir.blocks[B0].handlers.push(HandlerEdge {
            target,
            catch_type: Some("java/lang/Exception".into()),
            table_index: target.index(),
        });
    }
    assert_eq!(lower(&mut ir), 1);
    assert_eq!(ir.values[V0].local, ir.values[V1].local);
    assert_eq!(ir.blocks[B0].handlers.len(), 2);
}

#[test]
fn a_normal_edge_to_a_handler_does_not_define_the_caught_local() {
    let mut ir = fixture(vec![
        block(
            vec![
                assign(0, Definition(V0)),
                assign(1, Reference(V0)),
                assign(0, Definition(V2)),
            ],
            Terminator::Goto(B1),
        ),
        block(vec![], Terminator::Return(Some(V1))),
    ]);
    ir.values[V1].kind = ValueKind::CaughtException { handler: B1 };
    ir.blocks[B1].caught_exception = Some(V1);
    protected(&mut ir);
    assert_eq!(lower(&mut ir), 0);
}

#[test]
fn loop_reentry_does_not_make_old_value_ids_equal_to_new_contents() {
    let mut ir = fixture(vec![
        block(vec![assign(0, Definition(V0))], Terminator::Goto(B1)),
        block(
            vec![assign(1, Reference(V0)), Statement::Eval(V3)],
            Terminator::Branch {
                condition: Condition {
                    op: CompareOp::Eq,
                    left: V0,
                    right: V4,
                },
                then_target: B1,
                else_target: B2,
            },
        ),
        block(vec![], Terminator::Return(Some(V1))),
    ]);
    assert_eq!(lower(&mut ir), 0);
}

#[test]
fn this_types_and_distinct_debug_lifetimes_cannot_be_unified() {
    let sample = || {
        fixture(vec![block(
            vec![assign(0, Definition(V0)), assign(1, Reference(V0))],
            Terminator::Return(Some(V1)),
        )])
    };
    let mut receiver = sample();
    receiver.locals[L0].is_this = true;
    assert_eq!(lower(&mut receiver), 0);
    let mut narrow = sample();
    narrow.locals[L1].ty = JvmType::Byte;
    assert_eq!(lower(&mut narrow), 0);
    let mut debug = sample();
    for (index, local) in debug.locals.iter_mut().enumerate() {
        local.name_hint = Some(LocalVariable {
            index: index as u16,
            start_pc: 0,
            length: 10,
            name: format!("named{index}"),
            descriptor: "I".into(),
        });
    }
    assert_eq!(lower(&mut debug), 0);
}

fn bind_parameter(ir: &mut MethodIr, index: usize) {
    let local = LocalId::from_index(index);
    let value = ValueId::from_index(index);
    ir.locals[local].parameter = Some(index);
    ir.locals[local].name = format!("p{local}");
    ir.values[value].kind = ValueKind::Parameter {
        slot: index as u16,
        index: Some(index),
    };
    ir.parameters.push(value);
}

#[test]
fn parameter_entry_binding_owns_its_noninterfering_storage_group() {
    let mut ir = fixture(vec![
        block(
            vec![],
            Terminator::Branch {
                condition: Condition {
                    op: CompareOp::Eq,
                    left: V0,
                    right: V4,
                },
                then_target: B1,
                else_target: B2,
            },
        ),
        block(vec![assign(1, Reference(V0))], Terminator::Goto(B3)),
        block(vec![assign(1, Reference(V4))], Terminator::Goto(B3)),
        block(vec![], Terminator::Return(Some(V1))),
    ]);
    bind_parameter(&mut ir, 0);
    assert_eq!(lower(&mut ir), 1);
    assert_eq!(ir.values[V1].local, Some(L0));
    assert_eq!(ir.locals[L0].name, "p0");
    assert!(ir.blocks[B1].statements.is_empty());
    assert!(matches!(
        ir.blocks[B2].statements[0],
        Statement::Assign {
            local: L0,
            value: Reference(V4)
        }
    ));

    // Parameter identity wins even when another local has the smaller arena ID.
    let mut ir = fixture(vec![block(
        vec![assign(0, Reference(V1))],
        Terminator::Return(Some(V0)),
    )]);
    bind_parameter(&mut ir, 1);
    assert_eq!(lower(&mut ir), 1);
    assert_eq!(ir.values[V0].local, Some(L1));
}

#[test]
fn parameter_captures_and_distinct_argument_bindings_stay_separate() {
    let mut captured = fixture(vec![block(
        vec![assign(1, Reference(V0)), assign(1, Reference(V4))],
        Terminator::Return(Some(V0)),
    )]);
    bind_parameter(&mut captured, 0);
    assert_eq!(lower(&mut captured), 0);

    // Even a dead incoming argument keeps its own signature binding. Turning
    // a write to p1 into a write to p0 would also alias later argument reads.
    let mut distinct = fixture(vec![block(
        vec![assign(1, Reference(V0))],
        Terminator::Return(Some(V1)),
    )]);
    bind_parameter(&mut distinct, 0);
    bind_parameter(&mut distinct, 1);
    assert_eq!(lower(&mut distinct), 0);
    assert_ne!(distinct.values[V0].local, distinct.values[V1].local);
}

#[test]
fn parameter_entry_contents_reach_the_handler_until_a_successful_commit() {
    for intervening_throw in [false, true] {
        let mut statements = vec![assign(1, Reference(V0)), assign(1, Definition(V2))];
        if intervening_throw {
            statements.push(Statement::Eval(V2));
        }
        let mut ir = fixture(vec![
            block(statements, Terminator::Return(Some(V1))),
            block(vec![], Terminator::Return(Some(V0))),
        ]);
        bind_parameter(&mut ir, 0);
        ir.values[V2].local = Some(L1);
        protected(&mut ir);
        assert_eq!(lower(&mut ir) != 0, !intervening_throw);
        assert_eq!(
            ir.values[V0].local == ir.values[V1].local,
            !intervening_throw
        );
    }
}

#[test]
fn compact_liveness_transfer_matches_ordered_events_for_all_input_sets() {
    fn tree(seed: &mut u32, depth: usize) -> Effect {
        *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let choice = *seed;
        if depth == 0 {
            match choice % 4 {
                0 => Effect::Read(LocalId::from_index((choice as usize / 4) % 3)),
                1 => Effect::Write {
                    local: LocalId::from_index((choice as usize / 4) % 3),
                    copied: None,
                    value: Reference(V0),
                },
                2 => Effect::Throw,
                _ => Effect::Empty,
            }
        } else if choice & 1 == 0 {
            Effect::Sequence(vec![tree(seed, depth - 1), tree(seed, depth - 1)])
        } else {
            Effect::Choice(
                Box::new(tree(seed, depth - 1)),
                Box::new(tree(seed, depth - 1)),
            )
        }
    }
    let positions: IdVec<LocalId, _> = vec![Some(0), Some(1), Some(2)].into();
    let all = Bits(vec![7]);
    let mut seed = 1;
    for _ in 0..64 {
        let effect = tree(&mut seed, 4);
        let mut summary = Transfer {
            uses: Bits::zero(1),
            normal: all.clone(),
            exceptional: Bits::zero(1),
        };
        summarize(&effect, &mut summary, &positions, &all, &mut 10000).unwrap();
        for normal in 0..8 {
            for exceptional in 0..8 {
                let normal = Bits(vec![normal]);
                let exceptional = Bits(vec![exceptional]);
                let mut explicit = normal.clone();
                reverse(
                    &effect,
                    &mut explicit,
                    &exceptional,
                    &positions,
                    None,
                    &mut 10000,
                )
                .unwrap();
                let mut compact = summary.uses.clone();
                compact.union_intersection(&normal, &summary.normal);
                compact.union_intersection(&exceptional, &summary.exceptional);
                assert_eq!(compact, explicit, "{effect:?}");
            }
        }
    }
}
