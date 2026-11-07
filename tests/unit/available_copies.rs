use super::*;
use crate::cfg::HandlerEdge;
use crate::index::test_ids::*;
use crate::index::{BlockId, LocalId, ValueId};
use crate::ir::ValueUse;

fn block(statements: Vec<Statement>, terminator: Terminator) -> IrBlock {
    IrBlock {
        caught_exception: None,
        pc: 0,
        statements,
        terminator,
        handlers: vec![],
    }
}
fn copied() -> Statement {
    Statement::Assign {
        local: L1,
        value: ValueUse::Reference(V1),
    }
}
fn fixture(blocks: Vec<IrBlock>) -> MethodIr {
    let kinds = vec![
        ValueKind::Literal(Literal::Int(7)),
        ValueKind::Phi {
            inputs: vec![],
            slot: None,
        },
        ValueKind::Literal(Literal::Int(1)),
        ValueKind::Binary {
            op: BinaryOp::Add,
            left: V1,
            right: V2,
        },
        ValueKind::Phi {
            inputs: vec![],
            slot: None,
        },
        ValueKind::Parameter {
            slot: 0,
            index: Some(0),
        },
        ValueKind::Literal(Literal::Int(0)),
        ValueKind::Literal(Literal::Int(3)),
        ValueKind::Literal(Literal::Int(9)),
        ValueKind::InlineAssign {
            local: L0,
            value: V8,
        },
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
            locals: (0..3)
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
        for (entity, metadata) in ir.values.iter_mut().zip(vec![
            Some(0),
            Some(0),
            None,
            Some(0),
            Some(1),
            Some(2),
            None,
            None,
            None,
            None,
        ]) {
            entity.local = metadata.map(LocalId::from_index);
        }
        ir
    }
}
fn lower(ir: &mut MethodIr) {
    let mut raw = crate::ir::lift::RawIr::new(ir.clone());
    eliminate(&mut raw);
    raw.assert_statement_origins();
    *ir = raw.ir;
}

fn lower_stores(ir: &mut MethodIr) {
    let mut raw = crate::ir::lift::RawIr::new(ir.clone());
    eliminate_after_coalescing(&mut raw);
    raw.assert_statement_origins();
    *ir = raw.ir;
}

fn throwing_call(ir: &mut MethodIr, args: Vec<ValueId>) -> ValueId {
    ir.values.push(Value {
        local: None,
        kind: ValueKind::Call {
            kind: InvokeKind::Static,
            method: Member {
                owner: "Probe".into(),
                name: "effect".into(),
                descriptor: if args.is_empty() { "()I" } else { "(I)I" }.into(),
            },
            receiver: None,
            args,
        },
        ty: JvmType::Int,
        pc: 0,
        block: B1,
    })
}

fn handler_fixture() -> MethodIr {
    let mut ir = fixture(vec![
        block(
            vec![
                Statement::Assign {
                    local: L0,
                    value: ValueUse::Definition(V0),
                },
                copied(),
            ],
            Terminator::Goto(B1),
        ),
        block(vec![], Terminator::Goto(B3)),
        block(vec![], Terminator::Goto(B3)),
        block(vec![], Terminator::Return(None)),
    ]);
    ir.blocks[B1].handlers.push(HandlerEdge {
        target: B2,
        catch_type: None,
        table_index: 0,
    });
    ir
}

#[test]
fn handlers_meet_throw_sites_before_later_assignments_commit() {
    for throws_after_write in [false, true] {
        let mut ir = handler_fixture();
        let call = throwing_call(&mut ir, vec![]);
        ir.blocks[B1].statements = vec![
            Statement::Eval(call),
            Statement::Assign {
                local: L0,
                value: ValueUse::Reference(V8),
            },
        ];
        if throws_after_write {
            ir.blocks[B1].statements.push(Statement::Eval(call));
        }
        ir.blocks[B1].statements.push(copied());
        let facts = AvailableCopies::analyze(&ir);
        assert_eq!(facts.reads_same(B2, 0, L0, L1), !throws_after_write);
        assert!(facts.reads_literal(B2, 0, L1, &Literal::Int(7)));
        assert!(!facts.reads_literal(B2, 0, L0, &Literal::Int(9)));
    }
}

#[test]
fn a_throwing_assignment_retains_its_previous_destination_at_the_handler() {
    let mut ir = handler_fixture();
    let call = throwing_call(&mut ir, vec![]);
    ir.values[call].local = Some(L0);
    ir.blocks[B1].statements = vec![Statement::Assign {
        local: L0,
        value: ValueUse::Definition(call),
    }];
    let facts = AvailableCopies::analyze(&ir);
    assert!(facts.reads_same(B2, 0, L0, L1));
    assert!(facts.reads_literal(B2, 0, L0, &Literal::Int(7)));
    assert!(!facts.reads_same(B1, 1, L0, L1));
}

#[test]
fn embedded_writes_before_a_throw_split_their_old_alias_class() {
    let mut ir = handler_fixture();
    let call = throwing_call(&mut ir, vec![V9]);
    ir.blocks[B1].statements = vec![Statement::Eval(call)];
    let facts = AvailableCopies::analyze(&ir);
    assert!(!facts.reads_same(B2, 0, L0, L1));
    assert!(!facts.reads_literal(B2, 0, L0, &Literal::Int(7)));
    assert!(facts.reads_literal(B2, 0, L1, &Literal::Int(7)));
}

#[test]
fn an_embedded_conditional_write_is_met_on_both_exceptional_alternatives() {
    let mut ir = handler_fixture();
    let selected = ir.values.push(Value {
        local: None,
        kind: ValueKind::Select {
            condition: Condition {
                op: CompareOp::Eq,
                left: V5,
                right: V6,
            },
            then_value: V9,
            else_value: V8,
        },
        ty: JvmType::Int,
        pc: 0,
        block: B1,
    });
    let call = throwing_call(&mut ir, vec![selected]);
    ir.blocks[B1].statements = vec![Statement::Eval(call)];
    let facts = AvailableCopies::analyze(&ir);
    assert!(!facts.reads_same(B2, 0, L0, L1));
    assert!(!facts.reads_literal(B2, 0, L0, &Literal::Int(7)));
    assert!(facts.reads_literal(B2, 0, L1, &Literal::Int(7)));
}

#[test]
fn caught_binding_replaces_the_local_without_changing_its_old_snapshots() {
    let mut ir = handler_fixture();
    let call = throwing_call(&mut ir, vec![]);
    ir.blocks[B1].statements = vec![Statement::Eval(call)];
    let caught = ir.values.push(Value {
        local: Some(L0),
        kind: ValueKind::CaughtException { handler: B2 },
        ty: JvmType::Object("java/lang/Throwable".into()),
        pc: 0,
        block: B2,
    });
    ir.blocks[B2].caught_exception = Some(caught);
    let facts = AvailableCopies::analyze(&ir);
    assert!(!facts.reads_same(B2, 0, L0, L1));
    assert!(!facts.reads_literal(B2, 0, L0, &Literal::Int(7)));
    assert!(facts.reads_literal(B2, 0, L1, &Literal::Int(7)));
}

#[test]
fn an_inner_handler_preserves_allocated_snapshots_at_its_normal_rejoin() {
    let mut ir = handler_fixture();
    ir.values[V0].kind = ValueKind::New {
        class: "Object".into(),
        descriptor: "()V".into(),
        args: vec![],
        allocation_pc: 0,
    };
    for local in &mut ir.locals {
        local.ty = JvmType::Object("Object".into());
    }
    let call = throwing_call(&mut ir, vec![]);
    ir.blocks[B1].statements = vec![Statement::Eval(call)];
    let facts = AvailableCopies::analyze(&ir);
    for at in [B2, B3] {
        assert!(facts.reads_non_null(at, 0, L0));
        assert!(facts.reads_non_null(at, 0, L1));
        assert!(facts.reads_same(at, 0, L0, L1));
    }
    ir.values[V6].kind = ValueKind::Literal(Literal::Null);
    ir.blocks[B2].statements.push(Statement::Assign {
        local: L0,
        value: ValueUse::Reference(V6),
    });
    let facts = AvailableCopies::analyze(&ir);
    assert!(!facts.reads_non_null(B3, 0, L0));
    assert!(facts.reads_non_null(B3, 0, L1));
    assert!(!facts.reads_same(B3, 0, L0, L1));
}

#[test]
fn removed_copies_preserve_the_initialization_boundary_of_later_statements() {
    let mut raw =
        crate::ir::lift::RawIr::new(fixture(vec![block(vec![], Terminator::Return(None))]));
    raw.set_statements(
        B0,
        vec![
            (
                Statement::Assign {
                    local: L0,
                    value: ValueUse::Definition(V0),
                },
                5,
            ),
            (copied(), 10),
            (copied(), 11),
            (Statement::Eval(V3), 25),
        ],
    );
    raw.ir.values.push(Value {
        local: None,
        kind: ValueKind::Allocation {
            class: "Deferred".into(),
        },
        ty: JvmType::Object("Deferred".into()),
        pc: 20,
        block: B0,
    });

    eliminate(&mut raw);

    raw.assert_statement_origins();
    assert_eq!(raw.statement_pcs(B0), &[5, 10, 25]);
    let sites = crate::ir::initialization::requirements(&raw);
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0].statement_start, 2);
}

fn run(ir: &MethodIr, block: usize, locals: [i32; 3]) -> i32 {
    let mut block = BlockId::from_index(block);
    let mut locals: IdVec<LocalId, _> = locals.to_vec().into();
    fn value(ir: &MethodIr, locals: &mut IdVec<LocalId, i32>, id: ValueId, root: bool) -> i32 {
        if !root && let Some(local) = ir.values[id].local {
            return locals[local];
        }
        match ir.values[id].kind {
            ValueKind::Literal(Literal::Int(number)) => number,
            ValueKind::Binary {
                op: BinaryOp::Add,
                left,
                right,
            } => value(ir, locals, left, false).wrapping_add(value(ir, locals, right, false)),
            ValueKind::InlineAssign {
                local,
                value: source,
            } => {
                let result = value(ir, locals, source, true);
                locals[local] = result;
                result
            }
            _ => panic!("unexpected value"),
        }
    }
    for _ in 0..64 {
        for statement in &ir.blocks[block].statements {
            match *statement {
                Statement::Assign {
                    local,
                    value: source,
                } => {
                    locals[local] = value(
                        ir,
                        &mut locals,
                        source.value(),
                        matches!(source, ValueUse::Definition(_)),
                    )
                }
                Statement::Eval(source) => {
                    value(ir, &mut locals, source, true);
                }
                _ => panic!("unexpected statement"),
            }
        }
        match &ir.blocks[block].terminator {
            Terminator::Goto(target) => block = *target,
            Terminator::Return(Some(source)) => return value(ir, &mut locals, *source, false),
            Terminator::Branch {
                condition,
                then_target,
                else_target,
            } => {
                let left = value(ir, &mut locals, condition.left, false);
                let right = value(ir, &mut locals, condition.right, false);
                let yes = match condition.op {
                    CompareOp::Eq => left == right,
                    CompareOp::Lt => left < right,
                    _ => panic!("unexpected condition"),
                };
                block = if yes { *then_target } else { *else_target };
            }
            _ => panic!("unexpected terminator"),
        }
    }
    panic!("test program did not terminate")
}

#[test]
fn copies_available_on_all_normal_paths_are_removed() {
    let mut ir = fixture(vec![
        block(
            vec![
                Statement::Assign {
                    local: L0,
                    value: ValueUse::Definition(V0),
                },
                copied(),
            ],
            Terminator::Branch {
                condition: Condition {
                    op: CompareOp::Eq,
                    left: V5,
                    right: V6,
                },
                then_target: B1,
                else_target: B2,
            },
        ),
        block(vec![], Terminator::Goto(B3)),
        block(vec![copied()], Terminator::Goto(B3)),
        block(vec![copied()], Terminator::Return(Some(V4))),
    ]);
    lower(&mut ir);
    assert!(ir.blocks[B2].statements.is_empty() && ir.blocks[B3].statements.is_empty());
    for choice in [0, 1] {
        assert_eq!(run(&ir, 0, [0, 0, choice]), 7);
    }
}

#[test]
fn repeated_copy_elimination_joins_contents_independently_of_copy_direction() {
    let reversed = Statement::Assign {
        local: L0,
        value: ValueUse::Reference(V4),
    };
    let original = fixture(vec![
        block(
            vec![],
            Terminator::Branch {
                condition: Condition {
                    op: CompareOp::Eq,
                    left: V5,
                    right: V6,
                },
                then_target: B1,
                else_target: B2,
            },
        ),
        block(vec![copied(), copied()], Terminator::Goto(B3)),
        block(vec![reversed.clone(), reversed], Terminator::Goto(B3)),
        block(vec![copied()], Terminator::Return(Some(V4))),
    ]);
    for eliminate in [lower, lower_stores] {
        let mut lowered = original.clone();
        eliminate(&mut lowered);
        assert!(lowered.blocks[B3].statements.is_empty());
        for choice in [0, 1] {
            assert_eq!(
                run(&lowered, 0, [23, 17, choice]),
                run(&original, 0, [23, 17, choice])
            );
        }
    }
}

#[test]
fn repeated_copy_elimination_retains_snapshot_equality_after_source_overwrite() {
    let snapshot = Statement::Assign {
        local: L2,
        value: ValueUse::Reference(V1),
    };
    let restored = Statement::Assign {
        local: L1,
        value: ValueUse::Reference(V5),
    };
    let original = fixture(vec![block(
        vec![
            Statement::Assign {
                local: L0,
                value: ValueUse::Definition(V0),
            },
            copied(),
            copied(),
            snapshot.clone(),
            snapshot,
            Statement::Eval(V9),
            restored.clone(),
            restored,
        ],
        Terminator::Return(Some(V4)),
    )]);
    for eliminate in [lower, lower_stores] {
        let mut lowered = original.clone();
        eliminate(&mut lowered);
        assert_eq!(lowered.blocks[B0].statements.len(), 4);
        assert_eq!(run(&lowered, 0, [0, 0, 0]), 7);
        assert_eq!(run(&lowered, 0, [0, 0, 0]), run(&original, 0, [0, 0, 0]));
    }
}

#[test]
fn loop_local_writes_invalidate_the_same_ssa_source_on_reentry() {
    let mut ir = fixture(vec![
        block(
            vec![
                Statement::Assign {
                    local: L0,
                    value: ValueUse::Definition(V0),
                },
                copied(),
            ],
            Terminator::Goto(B1),
        ),
        block(
            vec![copied()],
            Terminator::Branch {
                condition: Condition {
                    op: CompareOp::Lt,
                    left: V1,
                    right: V7,
                },
                then_target: B2,
                else_target: B3,
            },
        ),
        block(
            vec![Statement::Assign {
                local: L0,
                value: ValueUse::Definition(V3),
            }],
            Terminator::Goto(B1),
        ),
        block(vec![], Terminator::Return(Some(V4))),
    ]);
    ir.values[V0].kind = ValueKind::Literal(Literal::Int(0));
    lower(&mut ir);
    assert_eq!(run(&ir, 0, [0, 0, 0]), 3);
}

#[test]
fn inline_assignment_kills_source_copy_facts() {
    let mut ir = fixture(vec![
        block(
            vec![
                Statement::Assign {
                    local: L0,
                    value: ValueUse::Definition(V0),
                },
                copied(),
            ],
            Terminator::Goto(B1),
        ),
        block(
            vec![Statement::Eval(V9), copied()],
            Terminator::Return(Some(V4)),
        ),
    ]);
    let facts = AvailableCopies::analyze(&ir);
    assert!(facts.reads_same(B1, 0, L0, L1));
    assert!(!facts.reads_same(B1, 1, L0, L1));
    assert!(facts.reads_same(B1, 2, L0, L1));
    lower(&mut ir);
    assert_eq!(run(&ir, 0, [0, 0, 0]), 9);
}

#[test]
fn read_equivalence_includes_a_single_snapshot_after_its_definition() {
    let ir = fixture(vec![block(
        vec![
            Statement::Assign {
                local: L0,
                value: ValueUse::Definition(V0),
            },
            copied(),
        ],
        Terminator::Return(Some(V4)),
    )]);
    let facts = AvailableCopies::analyze(&ir);
    assert!(!facts.reads_same(B0, 1, L0, L1));
    assert!(facts.reads_same(B0, 2, L0, L1));
}

#[test]
fn handler_entry_does_not_inherit_normal_completion_facts() {
    let mut ir = fixture(vec![
        block(
            vec![
                Statement::Assign {
                    local: L0,
                    value: ValueUse::Definition(V0),
                },
                copied(),
            ],
            Terminator::Goto(B1),
        ),
        block(vec![copied()], Terminator::Return(Some(V4))),
    ]);
    ir.blocks[B0].handlers.push(HandlerEdge {
        target: B1,
        catch_type: None,
        table_index: 0,
    });
    let call = throwing_call(&mut ir, vec![]);
    ir.blocks[B0].statements.insert(0, Statement::Eval(call));
    lower(&mut ir);
    assert_eq!(run(&ir, 1, [9, 3, 0]), 9);
}

#[test]
fn null_contents_flow_without_copy_candidates_and_stop_at_handlers() {
    let mut ir = fixture(vec![
        block(
            vec![Statement::Assign {
                local: L0,
                value: ValueUse::Definition(V0),
            }],
            Terminator::Goto(B1),
        ),
        block(vec![], Terminator::Return(None)),
        block(vec![], Terminator::Return(None)),
    ]);
    ir.values[V0].kind = ValueKind::Literal(Literal::Null);
    ir.values[V0].ty = JvmType::Object("java/lang/Object".into());
    ir.locals[L0].ty = JvmType::Array(Box::new(JvmType::Int));
    ir.blocks[B0].handlers.push(HandlerEdge {
        target: B2,
        catch_type: None,
        table_index: 0,
    });
    let facts = AvailableCopies::analyze(&ir);
    assert!(!facts.reads_literal(B0, 0, L0, &Literal::Null));
    assert!(facts.reads_literal(B0, 1, L0, &Literal::Null));
    assert!(facts.reads_literal(B1, 0, L0, &Literal::Null));
    assert!(!facts.reads_literal(B2, 0, L0, &Literal::Null));
}

#[test]
fn copied_literal_is_a_snapshot_not_the_value_ids_original_literal() {
    let ir = fixture(vec![block(
        vec![
            Statement::Assign {
                local: L0,
                value: ValueUse::Definition(V0),
            },
            copied(),
            Statement::Eval(V9),
            Statement::Assign {
                local: L1,
                value: ValueUse::Reference(V0),
            },
        ],
        Terminator::Return(Some(V4)),
    )]);
    let facts = AvailableCopies::analyze(&ir);
    assert!(facts.reads_literal(B0, 2, L1, &Literal::Int(7)));
    assert!(!facts.reads_literal(B0, 3, L0, &Literal::Int(7)));
    assert!(facts.reads_literal(B0, 3, L1, &Literal::Int(7)));
    // Value 0 still spells literal 7 in the immutable arena, but this RHS
    // reads the overwritten local. An embedded assignment remains a kill.
    assert!(!facts.reads_literal(B0, 4, L1, &Literal::Int(7)));
}

#[test]
fn literal_contents_require_the_same_fact_on_every_normal_path_and_iteration() {
    let mut ir = fixture(vec![
        block(
            vec![],
            Terminator::Branch {
                condition: Condition {
                    op: CompareOp::Eq,
                    left: V5,
                    right: V6,
                },
                then_target: B1,
                else_target: B2,
            },
        ),
        block(
            vec![Statement::Assign {
                local: L0,
                value: ValueUse::Reference(V6),
            }],
            Terminator::Goto(B3),
        ),
        block(
            vec![Statement::Assign {
                local: L0,
                value: ValueUse::Reference(V6),
            }],
            Terminator::Goto(B3),
        ),
        block(vec![], Terminator::Return(Some(V1))),
    ]);
    assert!(AvailableCopies::analyze(&ir).reads_literal(B3, 0, L0, &Literal::Int(0)));
    ir.blocks[B2].statements[0] = Statement::Assign {
        local: L0,
        value: ValueUse::Reference(V2),
    };
    let facts = AvailableCopies::analyze(&ir);
    assert!(!facts.reads_literal(B3, 0, L0, &Literal::Int(0)));
    assert!(!facts.reads_literal(B3, 0, L0, &Literal::Int(1)));

    let mut ir = fixture(vec![
        block(
            vec![Statement::Assign {
                local: L0,
                value: ValueUse::Definition(V0),
            }],
            Terminator::Goto(B1),
        ),
        block(
            vec![],
            Terminator::Branch {
                condition: Condition {
                    op: CompareOp::Eq,
                    left: V5,
                    right: V6,
                },
                then_target: B2,
                else_target: B3,
            },
        ),
        block(
            vec![Statement::Assign {
                local: L0,
                value: ValueUse::Reference(V8),
            }],
            Terminator::Goto(B1),
        ),
        block(vec![], Terminator::Return(Some(V1))),
    ]);
    let facts = AvailableCopies::analyze(&ir);
    assert!(!facts.reads_literal(B1, 0, L0, &Literal::Int(7)));
    assert!(!facts.reads_literal(B1, 0, L0, &Literal::Int(9)));
    ir.blocks[B2].statements[0] = Statement::Assign {
        local: L0,
        value: ValueUse::Definition(V0),
    };
    assert!(AvailableCopies::analyze(&ir).reads_literal(B1, 0, L0, &Literal::Int(7)));
}

#[test]
fn narrowing_truth_and_floating_conversions_are_not_assumed_identical() {
    for (literal, from, to, exact) in [
        (Literal::Int(127), JvmType::Int, JvmType::Byte, true),
        (Literal::Int(257), JvmType::Int, JvmType::Byte, false),
        (Literal::Int(1), JvmType::Int, JvmType::Boolean, true),
        (Literal::Int(2), JvmType::Int, JvmType::Boolean, false),
        (
            Literal::Int(16_777_217),
            JvmType::Int,
            JvmType::Float,
            false,
        ),
        (Literal::Long(1), JvmType::Long, JvmType::Int, false),
        (Literal::Double(1.5), JvmType::Double, JvmType::Float, false),
    ] {
        let mut ir = fixture(vec![block(
            vec![Statement::Assign {
                local: L0,
                value: ValueUse::Definition(V0),
            }],
            Terminator::Return(None),
        )]);
        ir.values[V0].kind = ValueKind::Literal(literal.clone());
        ir.values[V0].ty = from;
        ir.locals[L0].ty = to;
        assert_eq!(
            AvailableCopies::analyze(&ir).reads_literal(B0, 1, L0, &literal),
            exact,
            "{literal:?}"
        );
    }
}

#[test]
fn floating_literal_contents_compare_exact_bits() {
    for (stored, other, ty) in [
        (Literal::Float(-0.0), Literal::Float(0.0), JvmType::Float),
        (
            Literal::Float(f32::from_bits(0x7fc00001)),
            Literal::Float(f32::from_bits(0x7fc00002)),
            JvmType::Float,
        ),
        (Literal::Double(-0.0), Literal::Double(0.0), JvmType::Double),
        (
            Literal::Double(f64::from_bits(0x7ff8000000000001)),
            Literal::Double(f64::from_bits(0x7ff8000000000002)),
            JvmType::Double,
        ),
    ] {
        let mut ir = fixture(vec![block(
            vec![Statement::Assign {
                local: L0,
                value: ValueUse::Definition(V0),
            }],
            Terminator::Return(None),
        )]);
        ir.values[V0].kind = ValueKind::Literal(stored.clone());
        ir.values[V0].ty = ty.clone();
        ir.locals[L0].ty = ty;
        let facts = AvailableCopies::analyze(&ir);
        assert!(facts.reads_literal(B0, 1, L0, &stored));
        assert!(!facts.reads_literal(B0, 1, L0, &other));
    }
}

#[test]
fn repeated_stores_keep_entry_and_handler_definitions_and_respect_inline_writes() {
    let store = || Statement::Assign {
        local: L0,
        value: ValueUse::Definition(V0),
    };
    let mut ir = fixture(vec![
        block(
            vec![store(), store(), Statement::Eval(V9), store()],
            Terminator::Goto(B1),
        ),
        block(vec![store()], Terminator::Return(Some(V1))),
    ]);
    ir.locals[L0].needs_default = true;
    ir.blocks[B0].handlers.push(HandlerEdge {
        target: B1,
        catch_type: None,
        table_index: 0,
    });
    let call = throwing_call(&mut ir, vec![]);
    ir.blocks[B0].statements.insert(3, Statement::Eval(call));
    lower_stores(&mut ir);
    assert_eq!(ir.blocks[B0].statements.len(), 4);
    assert_eq!(ir.blocks[B1].statements.len(), 1);
    assert_eq!(run(&ir, 1, [9, 0, 0]), 7);
}

#[test]
fn repeated_stores_need_the_same_contents_on_every_branch() {
    let store = || Statement::Assign {
        local: L0,
        value: ValueUse::Definition(V0),
    };
    let mut ir = fixture(vec![
        block(
            vec![store()],
            Terminator::Branch {
                condition: Condition {
                    op: CompareOp::Eq,
                    left: V5,
                    right: V6,
                },
                then_target: B1,
                else_target: B2,
            },
        ),
        block(vec![store()], Terminator::Goto(B3)),
        block(vec![Statement::Eval(V9)], Terminator::Goto(B3)),
        block(vec![store()], Terminator::Return(Some(V1))),
    ]);
    lower_stores(&mut ir);
    assert!(ir.blocks[B1].statements.is_empty());
    assert_eq!(ir.blocks[B3].statements.len(), 1);
    for choice in [0, 1] {
        assert_eq!(run(&ir, 0, [0, 0, choice]), 7);
    }
}

#[test]
fn store_elimination_excludes_resolved_constants_and_distinguishes_zero_bits() {
    let store = || Statement::Assign {
        local: L0,
        value: ValueUse::Definition(V0),
    };
    for literal in [Literal::String(vec![65]), Literal::Class(JvmType::Int)] {
        let mut ir = fixture(vec![block(
            vec![store(), store()],
            Terminator::Return(None),
        )]);
        ir.values[V0].kind = ValueKind::Literal(literal);
        ir.values[V0].ty = JvmType::Object("java/lang/Object".into());
        ir.locals[L0].ty = ir.values[V0].ty.clone();
        lower_stores(&mut ir);
        assert_eq!(ir.blocks[B0].statements.len(), 2);
    }
    let mut ir = fixture(vec![block(
        vec![
            store(),
            store(),
            Statement::Assign {
                local: L0,
                value: ValueUse::Reference(V8),
            },
            store(),
        ],
        Terminator::Return(None),
    )]);
    ir.locals[L0].ty = JvmType::Float;
    ir.values[V0].ty = JvmType::Float;
    ir.values[V8].ty = JvmType::Float;
    ir.values[V0].kind = ValueKind::Literal(Literal::Float(0.0));
    ir.values[V8].kind = ValueKind::Literal(Literal::Float(-0.0));
    lower_stores(&mut ir);
    assert_eq!(ir.blocks[B0].statements.len(), 3);
}

#[test]
fn join_refines_copy_and_literal_content_classes_in_either_predecessor_order() {
    let mut ir = fixture(vec![
        block(
            vec![],
            Terminator::Branch {
                condition: Condition {
                    op: CompareOp::Eq,
                    left: V5,
                    right: V6,
                },
                then_target: B1,
                else_target: B2,
            },
        ),
        block(
            vec![
                Statement::Assign {
                    local: L0,
                    value: ValueUse::Reference(V5),
                },
                copied(),
            ],
            Terminator::Goto(B3),
        ),
        block(
            vec![
                Statement::Assign {
                    local: L0,
                    value: ValueUse::Reference(V6),
                },
                Statement::Assign {
                    local: L1,
                    value: ValueUse::Reference(V6),
                },
            ],
            Terminator::Goto(B3),
        ),
        block(vec![], Terminator::Return(Some(V4))),
    ]);
    for local in &mut ir.locals {
        local.ty = JvmType::Object("java/lang/Object".into());
    }
    for value in &mut ir.values {
        value.ty = JvmType::Object("java/lang/Object".into());
    }
    ir.values[V6].kind = ValueKind::Literal(Literal::Null);
    for _ in 0..2 {
        let facts = AvailableCopies::analyze(&ir);
        assert!(facts.reads_same(B3, 0, L0, L1));
        assert!(!facts.reads_literal(B3, 0, L0, &Literal::Null));
        assert!(!facts.reads_same(B3, 0, L0, L2));
        let first = ir.blocks[B1].clone();
        ir.blocks[B1] = ir.blocks[B2].clone();
        ir.blocks[B2] = first;
    }
    // Equality on only one predecessor is insufficient.
    ir.blocks[B2].statements[1] = Statement::Assign {
        local: L1,
        value: ValueUse::Reference(V5),
    };
    assert!(!AvailableCopies::analyze(&ir).reads_same(B3, 0, L0, L1));
}

#[test]
fn overwriting_a_content_class_representative_preserves_other_snapshots() {
    let mut facts = Rc::new(Facts::new());
    for (destination, source) in [(L1, L0), (L2, L0), (L3, L2)] {
        transfer(
            &mut facts,
            Action {
                local: destination,
                kind: ActionKind::Copy(source),
                statement: 0,
            },
        );
    }
    transfer(
        &mut facts,
        Action {
            local: L0,
            statement: 0,
            kind: ActionKind::Kill { non_null: false },
        },
    );
    assert_eq!(content(&facts, L1), content(&facts, L2));
    assert_eq!(content(&facts, L1), content(&facts, L3));
    assert_ne!(content(&facts, L1), content(&facts, L0));
    transfer(
        &mut facts,
        Action {
            local: L1,
            kind: ActionKind::Copy(L3),
            statement: 1,
        },
    );
    assert_eq!(content(&facts, L1), content(&facts, L3));
    assert_eq!(content(&facts, L2), content(&facts, L3));
}

#[test]
fn allocated_snapshots_keep_non_nullness_without_equating_fresh_objects() {
    let mut ir = fixture(vec![
        block(
            vec![
                Statement::Assign {
                    local: L0,
                    value: ValueUse::Definition(V0),
                },
                copied(),
                Statement::Assign {
                    local: L0,
                    value: ValueUse::Definition(V0),
                },
                Statement::Assign {
                    local: L2,
                    value: ValueUse::Reference(V4),
                },
                Statement::Assign {
                    local: L1,
                    value: ValueUse::Reference(V6),
                },
            ],
            Terminator::Return(None),
        ),
        block(vec![], Terminator::Return(None)),
    ]);
    ir.values[V0].kind = ValueKind::New {
        class: "Object".into(),
        descriptor: "()V".into(),
        args: vec![],
        allocation_pc: 0,
    };
    ir.values[V6].kind = ValueKind::Literal(Literal::Null);
    for local in &mut ir.locals {
        local.ty = JvmType::Object("Object".into());
    }
    ir.blocks[B0].handlers.push(HandlerEdge {
        target: B1,
        catch_type: None,
        table_index: 0,
    });
    let facts = AvailableCopies::analyze(&ir);
    assert!(facts.reads_same(B0, 2, L0, L1));
    assert!(!facts.reads_same(B0, 3, L0, L1));
    for local in [L0, L1] {
        assert!(facts.reads_non_null(B0, 3, local));
    }
    assert!(facts.reads_non_null(B0, 5, L2));
    assert!(facts.reads_literal(B0, 5, L1, &Literal::Null));
    assert!(!facts.reads_non_null(B0, 5, L1));
    assert!(!facts.reads_non_null(B1, 0, L0));
}

#[test]
fn non_null_joins_require_all_incoming_allocations() {
    let mut ir = fixture(vec![
        block(
            vec![],
            Terminator::Branch {
                condition: Condition {
                    op: CompareOp::Eq,
                    left: V5,
                    right: V6,
                },
                then_target: B1,
                else_target: B2,
            },
        ),
        block(
            vec![Statement::Assign {
                local: L0,
                value: ValueUse::Definition(V0),
            }],
            Terminator::Goto(B3),
        ),
        block(
            vec![Statement::Assign {
                local: L0,
                value: ValueUse::Definition(V0),
            }],
            Terminator::Goto(B3),
        ),
        block(vec![], Terminator::Return(None)),
    ]);
    ir.values[V0].kind = ValueKind::New {
        class: "Object".into(),
        descriptor: "()V".into(),
        args: vec![],
        allocation_pc: 0,
    };
    ir.values[V6].kind = ValueKind::Literal(Literal::Null);
    ir.locals[L0].ty = JvmType::Object("Object".into());
    assert!(AvailableCopies::analyze(&ir).reads_non_null(B3, 0, L0));
    ir.blocks[B2].statements[0] = Statement::Assign {
        local: L0,
        value: ValueUse::Reference(V6),
    };
    assert!(!AvailableCopies::analyze(&ir).reads_non_null(B3, 0, L0));
}

#[test]
fn content_transfers_match_concrete_snapshots_and_joins() {
    let mut random = 0x5938_34cd_u64;
    let mut next = || {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        random as usize
    };
    let mut facts = [Rc::new(Facts::new()), Rc::new(Facts::new())];
    let mut concrete = [[0_i64, 1, 2, 3, 4, 5, 6, 7]; 2];
    for step in 0..2000 {
        for (facts, locals) in facts.iter_mut().zip(&mut concrete) {
            let destination = next() % locals.len();
            let source = next() % locals.len();
            let action = match next() % 3 {
                0 => {
                    locals[destination] = locals[source];
                    Action {
                        local: LocalId::from_index(destination),
                        kind: ActionKind::Copy(LocalId::from_index(source)),
                        statement: step,
                    }
                }
                1 => {
                    locals[destination] = -1 - source as i64;
                    Action {
                        local: LocalId::from_index(destination),
                        kind: ActionKind::Literal(source),
                        statement: step,
                    }
                }
                _ => {
                    locals[destination] = step as i64 + 8;
                    Action {
                        local: LocalId::from_index(destination),
                        statement: step,
                        kind: ActionKind::Kill {
                            non_null: next() % 2 == 0,
                        },
                    }
                }
            };
            transfer(facts, action);
            for left in 0..locals.len() {
                for right in 0..locals.len() {
                    assert_eq!(
                        content(facts, LocalId::from_index(left))
                            == content(facts, LocalId::from_index(right)),
                        locals[left] == locals[right]
                    );
                }
            }
        }
        let joined = common_contents(&facts[0], &facts[1]);
        assert_eq!(joined, common_contents(&facts[1], &facts[0]));
        for left in 0..8 {
            let read = content(&joined, LocalId::from_index(left));
            if let Content::Literal(literal) = read {
                assert!(
                    concrete
                        .iter()
                        .all(|locals| locals[left] == -1 - literal as i64)
                );
            }
            for right in 0..8 {
                assert_eq!(
                    read == content(&joined, LocalId::from_index(right)),
                    concrete.iter().all(|locals| locals[left] == locals[right])
                );
            }
        }
    }
}

#[test]
fn copy_cycles_converge_by_contents_despite_rotating_storage_representatives() {
    let mut ir = fixture(vec![
        block(
            vec![
                Statement::Assign {
                    local: L0,
                    value: ValueUse::Reference(V5),
                },
                Statement::Assign {
                    local: L3,
                    value: ValueUse::Reference(V5),
                },
                Statement::Assign {
                    local: L2,
                    value: ValueUse::Reference(V0),
                },
                Statement::Assign {
                    local: L1,
                    value: ValueUse::Reference(V5),
                },
            ],
            Terminator::Goto(B1),
        ),
        block(
            vec![
                Statement::Assign {
                    local: L0,
                    value: ValueUse::Reference(V4),
                },
                Statement::Assign {
                    local: L1,
                    value: ValueUse::Reference(V5),
                },
            ],
            Terminator::Branch {
                condition: Condition {
                    op: CompareOp::Eq,
                    left: V0,
                    right: V7,
                },
                then_target: B1,
                else_target: B2,
            },
        ),
        block(vec![], Terminator::Return(Some(V4))),
    ]);
    ir.locals.push(Local {
        name_hint: None,
        name: "v3".into(),
        ty: JvmType::Int,
        parameter: None,
        is_this: false,
        needs_default: false,
    });
    let facts = AvailableCopies::analyze(&ir);
    for local in [L1, L2, L3] {
        assert!(facts.reads_same(B1, 0, L0, local));
        assert!(facts.reads_same(B2, 0, L0, local));
    }
}

#[test]
fn assignment_views_compare_current_contents_at_one_exact_point() {
    let ir = fixture(vec![block(
        vec![
            Statement::Assign {
                local: L0,
                value: ValueUse::Definition(V0),
            },
            Statement::Assign {
                local: L1,
                value: ValueUse::Reference(V8),
            },
            Statement::Assign {
                local: L1,
                value: ValueUse::Reference(V0),
            },
            Statement::Eval(V9),
            Statement::Assign {
                local: L1,
                value: ValueUse::Reference(V0),
            },
        ],
        Terminator::Return(Some(V4)),
    )]);
    let facts = AvailableCopies::analyze(&ir);
    let first = facts.assignment_contents(&ir, B0, 2).unwrap();
    assert!(first.held_by(L0));
    assert!(!first.held_by(L1));
    assert!(!first.held_by(L2));
    let after_mutation = facts.assignment_contents(&ir, B0, 4).unwrap();
    assert!(after_mutation.held_by(L0));
    assert!(!after_mutation.held_by(L1));
    assert!(facts.assignment_contents(&ir, B0, 3).is_none());
}

#[test]
fn interference_views_exclude_resolved_literals_and_nonidentity_conversions() {
    for (literal, from, to, eligible) in [
        (Literal::Int(127), JvmType::Int, JvmType::Byte, true),
        (Literal::Int(257), JvmType::Int, JvmType::Byte, false),
        (Literal::Int(2), JvmType::Int, JvmType::Boolean, false),
        (Literal::Double(1.5), JvmType::Double, JvmType::Float, false),
        (
            Literal::String(vec![65]),
            JvmType::Object("java/lang/String".into()),
            JvmType::Object("java/lang/String".into()),
            false,
        ),
        (
            Literal::Class(JvmType::Int),
            JvmType::Object("java/lang/Class".into()),
            JvmType::Object("java/lang/Class".into()),
            false,
        ),
    ] {
        let mut ir = fixture(vec![block(
            vec![Statement::Assign {
                local: L0,
                value: ValueUse::Definition(V0),
            }],
            Terminator::Return(None),
        )]);
        ir.values[V0].kind = ValueKind::Literal(literal);
        ir.values[V0].ty = from;
        ir.locals[L0].ty = to;
        assert_eq!(
            has_assignment_contents(&ir, &ir.blocks[B0].statements[0]),
            eligible
        );
        assert_eq!(
            AvailableCopies::analyze(&ir)
                .assignment_contents(&ir, B0, 0)
                .is_some(),
            eligible
        );
    }
    let mut ir = fixture(vec![block(
        vec![Statement::Assign {
            local: L1,
            value: ValueUse::Reference(V0),
        }],
        Terminator::Return(None),
    )]);
    ir.locals[L1].ty = JvmType::Byte;
    assert!(!has_assignment_contents(&ir, &ir.blocks[B0].statements[0]));
}
