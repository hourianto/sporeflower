use super::*;
use crate::cfg::HandlerEdge;
use crate::classfile::JvmType;
use crate::index::{BlockId, LocalId, ValueId};
use crate::ir::{InvokeKind, IrBlock, Local, Member, Statement, Terminator, Value, ValueKind};
use crate::structure::Predicate;
use std::collections::BTreeSet;

fn sequential_cleanups(count: usize) -> MethodIr {
    let source = LocalId::from_index(0);
    let copied = LocalId::from_index(1);
    let mut ir = MethodIr {
        entry: BlockId::from_index(0),
        blocks: Default::default(),
        values: Default::default(),
        locals: (0..count + 2)
            .map(|index| Local {
                name_hint: None,
                name: format!("v{index}"),
                ty: if index < 2 {
                    JvmType::Int
                } else {
                    JvmType::Object("java/lang/Throwable".into())
                },
                parameter: (index == 0).then_some(0),
                is_this: false,
                needs_default: false,
            })
            .collect(),
        parameters: vec![ValueId::from_index(0)],
        return_type: JvmType::Void,
        initialization_sites: vec![],
    };
    let parameter = ir.values.push(Value {
        local: Some(source),
        kind: ValueKind::Parameter {
            slot: 0,
            index: Some(0),
        },
        ty: JvmType::Int,
        pc: 0,
        block: ir.entry,
    });
    let reference = ir.values.push(Value {
        local: Some(copied),
        kind: ValueKind::Phi {
            inputs: vec![],
            slot: None,
        },
        ty: JvmType::Int,
        pc: 0,
        block: ir.entry,
    });
    ir.blocks.push(IrBlock {
        caught_exception: None,
        pc: 0,
        statements: vec![Statement::Assign {
            local: copied,
            value: crate::ir::ValueUse::Reference(parameter),
        }],
        terminator: Terminator::Goto(BlockId::from_index(1)),
        handlers: vec![],
    });
    for index in 0..count {
        let body = ir.blocks.next_id();
        let normal = BlockId::from_index(body.index() + 1);
        let handler = BlockId::from_index(body.index() + 2);
        let next = BlockId::from_index(body.index() + 3);
        let effect = call(&mut ir, body, "effect", vec![]);
        let normal_cleanup = call(&mut ir, normal, "cleanup", vec![reference]);
        let caught_cleanup = call(&mut ir, handler, "cleanup", vec![parameter]);
        let caught = ir.values.push(Value {
            local: Some(LocalId::from_index(index + 2)),
            kind: ValueKind::CaughtException { handler },
            ty: JvmType::Object("java/lang/Throwable".into()),
            pc: handler.index() as u32,
            block: handler,
        });
        ir.blocks.push(IrBlock {
            caught_exception: None,
            pc: body.index() as u32,
            statements: vec![Statement::Eval(effect)],
            terminator: Terminator::Goto(normal),
            handlers: vec![HandlerEdge {
                target: handler,
                catch_type: None,
                table_index: index,
            }],
        });
        ir.blocks.push(IrBlock {
            caught_exception: None,
            pc: normal.index() as u32,
            statements: vec![Statement::Eval(normal_cleanup)],
            terminator: Terminator::Goto(next),
            handlers: vec![],
        });
        ir.blocks.push(IrBlock {
            caught_exception: Some(caught),
            pc: handler.index() as u32,
            statements: vec![Statement::Eval(caught_cleanup)],
            terminator: Terminator::Throw {
                value: caught,
                pc: handler.index() as u32,
            },
            handlers: vec![],
        });
    }
    ir.blocks.push(IrBlock {
        caught_exception: None,
        pc: ir.blocks.len() as u32,
        statements: vec![],
        terminator: Terminator::Return(None),
        handlers: vec![],
    });
    ir
}

fn call(ir: &mut MethodIr, block: BlockId, name: &str, args: Vec<ValueId>) -> ValueId {
    ir.values.push(Value {
        local: None,
        kind: ValueKind::Call {
            kind: InvokeKind::Static,
            method: Member {
                owner: "CleanupProbe".into(),
                name: name.into(),
                descriptor: if args.is_empty() { "()V" } else { "(I)V" }.into(),
            },
            receiver: None,
            args,
        },
        ty: JvmType::Void,
        pc: block.index() as u32,
        block,
    })
}

#[test]
fn independent_cleanup_candidates_share_one_lazy_copy_analysis() {
    for count in [0, 2] {
        let prepared = crate::structure::PreparedIr::new(sequential_cleanups(count));
        let analysis = Analysis::new(prepared.ir());
        // The two copies read different locals. Their equality depends on the
        // contents established before both try scopes, not SSA identity.
        let recovered = crate::structure::cleanup::recover(&analysis, None);
        for index in 0..count {
            let handler_pc = (3 + 3 * index) as u32;
            assert!(recovered.regions.cleanups.consumes(handler_pc, handler_pc));
        }
        let tree = crate::structure::finalize_source(prepared.ir(), recovered.lower(None).unwrap())
            .unwrap();
        assert_eq!(
            crate::structure::tree::walk(&tree.body)
                .filter(|statement| matches!(statement, Stmt::TryFinally { .. }))
                .count(),
            count
        );
        assert_eq!(analysis.available.get().is_some(), count != 0);
    }
}

#[test]
fn cleanup_ownership_includes_pure_branches_and_loops_before_the_only_effect() {
    let mut ir = sequential_cleanups(1);
    let effect = BlockId::from_index(1);
    let normal = BlockId::from_index(2);
    let handler = BlockId::from_index(3);
    let head = ir.blocks.next_id();
    let loop_head = BlockId::from_index(head.index() + 1);
    let zero = ir.values.push(Value {
        local: None,
        kind: ValueKind::Literal(crate::ir::Literal::Int(0)),
        ty: JvmType::Int,
        pc: 5,
        block: head,
    });
    let condition = crate::ir::Condition {
        op: crate::ir::CompareOp::Eq,
        left: ir.parameters[0],
        right: zero,
    };
    ir.blocks[ir.entry].terminator = Terminator::Goto(head);
    for (then_target, else_target) in [(effect, loop_head), (loop_head, normal)] {
        ir.blocks.push(IrBlock {
            caught_exception: None,
            pc: ir.blocks.len() as u32,
            statements: vec![],
            terminator: Terminator::Branch {
                condition: condition.clone(),
                then_target,
                else_target,
            },
            handlers: ir.blocks[effect].handlers.clone(),
        });
    }
    let prepared = crate::structure::PreparedIr::new(ir);
    let analysis = Analysis::new(prepared.ir());
    // The throwing arm alone cannot own the shared cleanup: the pure loop
    // also reaches it. Discovery must retain the whole lexical table scope.
    let recovered = crate::structure::cleanup::recover(&analysis, None);
    assert!(
        recovered
            .regions
            .cleanups
            .consumes(handler.index() as u32, handler.index() as u32)
    );
    assert_eq!(
        recovered
            .regions
            .replacements
            .values()
            .flat_map(|body| { crate::structure::tree::walk(body) })
            .filter(|statement| matches!(statement, Stmt::TryFinally { .. }))
            .count(),
        1
    );
}

#[test]
fn partial_cleanup_keeps_the_shared_exceptional_witness_and_rethrow_fact() {
    let mut ir = sequential_cleanups(1);
    let protected = BlockId::from_index(1);
    let normal = BlockId::from_index(2);
    let handler = BlockId::from_index(3);
    let follow = BlockId::from_index(4);
    let caught = ir.blocks.next_id();
    let copied = BlockId::from_index(caught.index() + 1);
    let Statement::Eval(value) = ir.blocks[normal].statements[0] else {
        panic!()
    };
    let ValueKind::Call { method, .. } = &mut ir.values[value].kind else {
        panic!()
    };
    method.name = "differentCleanup".into();
    let exception = ir.values.push(Value {
        local: None,
        kind: ValueKind::CaughtException { handler: caught },
        ty: JvmType::Object("java/lang/Exception".into()),
        pc: caught.index() as u32,
        block: caught,
    });
    let effect = call(&mut ir, caught, "caughtEffect", vec![]);
    let parameter = ir.parameters[0];
    let cleanup = call(&mut ir, copied, "cleanup", vec![parameter]);
    ir.blocks.push(IrBlock {
        caught_exception: Some(exception),
        pc: caught.index() as u32,
        statements: vec![Statement::Eval(effect)],
        terminator: Terminator::Goto(copied),
        handlers: ir.blocks[protected].handlers.clone(),
    });
    ir.blocks.push(IrBlock {
        caught_exception: None,
        pc: copied.index() as u32,
        statements: vec![Statement::Eval(cleanup)],
        terminator: Terminator::Goto(follow),
        handlers: vec![],
    });
    ir.blocks[protected].handlers.insert(
        0,
        HandlerEdge {
            target: caught,
            catch_type: Some("java/lang/Exception".into()),
            table_index: 1,
        },
    );
    let prepared = crate::structure::PreparedIr::new(ir);
    let analysis = Analysis::new(prepared.ir());
    let recovered = crate::structure::cleanup::recover(&analysis, None);
    assert!(
        !recovered
            .regions
            .cleanups
            .consumes(handler.index() as u32, handler.index() as u32)
    );
    assert!(matches!(
        recovered.regions.flow.blocks[handler].end,
        crate::structure::FlowEnd::Throw
    ));
    assert!(
        recovered.regions.flow.blocks[protected]
            .handlers
            .iter()
            .any(|edge| edge.target == handler)
    );
    let body = &recovered.regions.replacements[&caught];
    assert_eq!(
        crate::structure::tree::walk(body)
            .filter(|statement| matches!(statement, Stmt::TryFinally { handler: owner, .. } if *owner == handler))
            .count(),
        1
    );
}

#[test]
fn shared_cleanup_ownership_does_not_depend_on_which_scope_matches_first() {
    for matching_first in [true, false] {
        let mut ir = sequential_cleanups(2);
        let first = BlockId::from_index(1);
        let second = BlockId::from_index(4);
        let handler = BlockId::from_index(3);
        ir.blocks[second].handlers = ir.blocks[first].handlers.clone();
        let different = BlockId::from_index(if matching_first { 5 } else { 2 });
        let Statement::Eval(value) = ir.blocks[different].statements[0] else {
            panic!()
        };
        let ValueKind::Call { method, .. } = &mut ir.values[value].kind else {
            panic!()
        };
        method.name = "differentCleanup".into();
        let analysis = Analysis::new(&ir);
        let recovered = crate::structure::cleanup::recover(&analysis, None);
        let (owned, retained) = if matching_first {
            (first, second)
        } else {
            (second, first)
        };
        assert!(
            recovered.regions.replacements.get(&owned).is_some_and(|body|
                crate::structure::tree::walk(body)
                    .any(|statement| matches!(statement, Stmt::TryFinally { handler: owner, .. } if *owner == handler))),
            "matching first: {matching_first}"
        );
        assert!(
            recovered.regions.flow.blocks[retained]
                .handlers
                .iter()
                .any(|edge| edge.target == handler)
        );
        assert!(matches!(
            recovered.regions.flow.blocks[handler].end,
            crate::structure::FlowEnd::Throw
        ));
        assert!(!recovered.regions.cleanups.consumes(3, 3));
    }
}

#[test]
fn shared_cleanup_does_not_split_a_branch_out_of_its_original_try() {
    let mut ir = sequential_cleanups(2);
    let first = BlockId::from_index(1);
    let normal = BlockId::from_index(2);
    let handler = BlockId::from_index(3);
    let second = BlockId::from_index(4);
    ir.blocks[second].handlers = ir.blocks[first].handlers.clone();
    let Statement::Eval(value) = ir.blocks[normal].statements[0] else {
        panic!()
    };
    let ValueKind::Call { method, .. } = &mut ir.values[value].kind else {
        panic!()
    };
    method.name = "differentCleanup".into();
    ir.blocks[normal].terminator = Terminator::Return(None);
    let zero = ir.values.push(Value {
        local: None,
        kind: ValueKind::Literal(crate::ir::Literal::Int(0)),
        ty: JvmType::Int,
        pc: 1,
        block: first,
    });
    ir.blocks[first].terminator = Terminator::Branch {
        condition: crate::ir::Condition {
            op: crate::ir::CompareOp::Eq,
            left: ir.parameters[0],
            right: zero,
        },
        then_target: normal,
        else_target: second,
    };
    // Both arms belong to one original try. Extracting only the second arm
    // would keep the canonical cleanup and move its sole copy into a finally,
    // introducing another try without consolidating the existing scope.
    let analysis = Analysis::new(&ir);
    let recovered = crate::structure::cleanup::recover(&analysis, None);
    assert!(recovered.regions.replacements.is_empty());
    assert!(
        recovered.regions.flow.blocks[second]
            .handlers
            .iter()
            .any(|edge| edge.target == handler)
    );
}

#[test]
fn shared_cleanup_starts_at_a_pure_catch_guard_before_its_throwing_arms() {
    let mut ir = sequential_cleanups(1);
    let protected = BlockId::from_index(1);
    let normal = BlockId::from_index(2);
    let handler = BlockId::from_index(3);
    let follow = BlockId::from_index(4);
    let caught = ir.blocks.next_id();
    let left = BlockId::from_index(caught.index() + 1);
    let right = BlockId::from_index(caught.index() + 2);
    let copied = BlockId::from_index(caught.index() + 3);
    let Statement::Eval(value) = ir.blocks[normal].statements[0] else {
        panic!()
    };
    let ValueKind::Call { method, .. } = &mut ir.values[value].kind else {
        panic!()
    };
    method.name = "differentCleanup".into();
    let exception = ir.values.push(Value {
        local: None,
        kind: ValueKind::CaughtException { handler: caught },
        ty: JvmType::Object("java/lang/Exception".into()),
        pc: caught.index() as u32,
        block: caught,
    });
    let zero = ir.values.push(Value {
        local: None,
        kind: ValueKind::Literal(crate::ir::Literal::Int(0)),
        ty: JvmType::Int,
        pc: caught.index() as u32,
        block: caught,
    });
    ir.blocks.push(IrBlock {
        caught_exception: Some(exception),
        pc: caught.index() as u32,
        statements: vec![],
        terminator: Terminator::Branch {
            condition: crate::ir::Condition {
                op: crate::ir::CompareOp::Eq,
                left: ir.parameters[0],
                right: zero,
            },
            then_target: left,
            else_target: right,
        },
        handlers: ir.blocks[protected].handlers.clone(),
    });
    for block in [left, right, copied] {
        let parameter = ir.parameters[0];
        let effect = if block == copied {
            call(&mut ir, block, "cleanup", vec![parameter])
        } else {
            call(&mut ir, block, "caughtEffect", vec![])
        };
        ir.blocks.push(IrBlock {
            caught_exception: None,
            pc: block.index() as u32,
            statements: vec![Statement::Eval(effect)],
            terminator: Terminator::Goto(if block == copied { follow } else { copied }),
            handlers: if block == copied {
                vec![]
            } else {
                ir.blocks[protected].handlers.clone()
            },
        });
    }
    ir.blocks[protected].handlers.insert(
        0,
        HandlerEdge {
            target: caught,
            catch_type: Some("java/lang/Exception".into()),
            table_index: 1,
        },
    );
    let analysis = Analysis::new(&ir);
    let recovered = crate::structure::cleanup::recover(&analysis, None);
    let scope = &recovered.regions.replacements[&caught];
    assert_eq!(
        crate::structure::tree::walk(scope)
            .filter(|statement| matches!(statement, Stmt::TryFinally { .. }))
            .count(),
        1
    );
    assert!(!recovered.regions.replacements.contains_key(&left));
    assert!(!recovered.regions.replacements.contains_key(&right));
    assert!(
        !recovered
            .regions
            .cleanups
            .consumes(handler.index() as u32, handler.index() as u32)
    );
}

#[test]
fn exact_cleanup_loops_keep_guards_whose_locals_the_cleanup_updates() {
    let mut ir = sequential_cleanups(1);
    let normal = BlockId::from_index(2);
    let handler = BlockId::from_index(3);
    let returned = BlockId::from_index(4);
    let normal_body = BlockId::from_index(5);
    let guard = BlockId::from_index(6);
    let rethrow = BlockId::from_index(7);
    let cleanup_body = BlockId::from_index(8);
    let Terminator::Throw { value: caught, .. } = ir.blocks[handler].terminator else {
        panic!()
    };
    let literals: Vec<_> = [1, 0]
        .into_iter()
        .map(|number| {
            ir.values.push(Value {
                local: None,
                kind: ValueKind::Literal(crate::ir::Literal::Int(number)),
                ty: JvmType::Int,
                pc: 0,
                block: ir.entry,
            })
        })
        .collect();
    let local = LocalId::from_index(1);
    ir.blocks[ir.entry].statements = vec![Statement::Assign {
        local,
        value: crate::ir::ValueUse::Reference(literals[0]),
    }];
    let condition = crate::ir::Condition {
        op: crate::ir::CompareOp::Eq,
        left: ValueId::from_index(1),
        right: literals[1],
    };
    ir.blocks[normal].statements.clear();
    ir.blocks[normal].terminator = Terminator::Branch {
        condition: condition.clone(),
        then_target: returned,
        else_target: normal_body,
    };
    ir.blocks[handler].statements.clear();
    ir.blocks[handler].terminator = Terminator::Goto(guard);
    for (block, terminator, writes) in [
        (normal_body, Terminator::Goto(normal), true),
        (
            guard,
            Terminator::Branch {
                condition,
                then_target: rethrow,
                else_target: cleanup_body,
            },
            false,
        ),
        (
            rethrow,
            Terminator::Throw {
                value: caught,
                pc: 7,
            },
            false,
        ),
        (cleanup_body, Terminator::Goto(guard), true),
    ] {
        let mut statements = vec![];
        if writes {
            let effect = call(&mut ir, block, "close", vec![]);
            statements.push(Statement::Eval(effect));
            statements.push(Statement::Assign {
                local,
                value: crate::ir::ValueUse::Reference(literals[1]),
            });
        }
        ir.blocks.push(IrBlock {
            caught_exception: None,
            pc: block.index() as u32,
            statements,
            terminator,
            handlers: vec![],
        });
    }
    let prepared = crate::structure::PreparedIr::new(ir);
    let facts = crate::structure::cleanup_facts(&prepared);
    // Both guards start with the known value1, then observe the cleanup's
    // write on the next iteration. Exact correspondence needs no stability
    // assumption that would incorrectly specialize either guard forever.
    assert!(facts.consumes(3, 7));
}

#[test]
fn specialized_noop_exits_need_complete_ownership_or_physical_copies() {
    // A complete scope may have only no-op normal exits, or combine them with
    // physical cleanup. Retaining an allocated resource on success instead
    // prevents whole ownership; its null early exit cannot establish a scope.
    for (allocated, physical_copy, owned) in [
        (false, false, true),
        (true, true, true),
        (true, false, false),
    ] {
        let mut ir = sequential_cleanups(1);
        let protected = BlockId::from_index(1);
        let normal = BlockId::from_index(2);
        let handler = BlockId::from_index(3);
        let rethrow = ir.blocks.next_id();
        let cleanup = BlockId::from_index(rethrow.index() + 1);
        let early_return = BlockId::from_index(rethrow.index() + 2);
        let Terminator::Throw { value: caught, .. } = ir.blocks[handler].terminator else {
            panic!()
        };
        let reference_type = JvmType::Object("java/lang/Object".into());
        ir.locals[LocalId::from_index(1)].ty = reference_type.clone();
        ir.values[ValueId::from_index(1)].ty = reference_type.clone();
        let zero = ir.values.push(Value {
            local: None,
            kind: ValueKind::Literal(crate::ir::Literal::Int(0)),
            ty: JvmType::Int,
            pc: 0,
            block: ir.entry,
        });
        let contents: Vec<_> = [
            ValueKind::Literal(crate::ir::Literal::Null),
            ValueKind::New {
                class: "java/lang/Object".into(),
                descriptor: "()V".into(),
                args: vec![],
                allocation_pc: 1,
            },
        ]
        .into_iter()
        .map(|kind| {
            let allocation = matches!(kind, ValueKind::New { .. });
            ir.values.push(Value {
                local: None,
                kind,
                ty: reference_type.clone(),
                pc: u32::from(allocation),
                block: if allocation { protected } else { ir.entry },
            })
        })
        .collect();
        ir.blocks[ir.entry].statements = vec![Statement::Assign {
            local: LocalId::from_index(1),
            value: crate::ir::ValueUse::Reference(contents[0]),
        }];
        ir.blocks[ir.entry].handlers = ir.blocks[protected].handlers.clone();
        ir.blocks[ir.entry].terminator = Terminator::Branch {
            condition: crate::ir::Condition {
                op: crate::ir::CompareOp::Eq,
                left: ir.parameters[0],
                right: zero,
            },
            then_target: early_return,
            else_target: protected,
        };
        ir.blocks[protected].statements.insert(
            0,
            Statement::Assign {
                local: LocalId::from_index(1),
                value: crate::ir::ValueUse::Reference(contents[usize::from(allocated)]),
            },
        );
        let close = call(&mut ir, normal, "close", vec![]);
        ir.blocks[normal].statements = if physical_copy {
            vec![Statement::Eval(close)]
        } else {
            vec![]
        };
        ir.blocks[handler].statements.clear();
        ir.blocks[handler].terminator = Terminator::Branch {
            condition: crate::ir::Condition {
                op: crate::ir::CompareOp::Eq,
                left: ValueId::from_index(1),
                right: contents[0],
            },
            then_target: rethrow,
            else_target: cleanup,
        };
        ir.blocks.push(IrBlock {
            caught_exception: None,
            pc: rethrow.index() as u32,
            statements: vec![],
            terminator: Terminator::Throw {
                value: caught,
                pc: rethrow.index() as u32,
            },
            handlers: vec![],
        });
        let close = call(&mut ir, cleanup, "close", vec![]);
        ir.blocks.push(IrBlock {
            caught_exception: None,
            pc: cleanup.index() as u32,
            statements: vec![Statement::Eval(close)],
            terminator: Terminator::Goto(rethrow),
            handlers: vec![],
        });
        ir.blocks.push(IrBlock {
            caught_exception: None,
            pc: early_return.index() as u32,
            statements: vec![],
            terminator: Terminator::Return(None),
            handlers: vec![],
        });
        let analysis = Analysis::new(&ir);
        let recovered = crate::structure::cleanup::recover(&analysis, None);
        assert_eq!(
            recovered.regions.cleanups.consumes(3, 5),
            owned,
            "{:#?}",
            recovered.regions.replacements
        );
        if !owned {
            assert!(recovered.regions.replacements.is_empty());
        }
    }
}

#[test]
fn cleanup_completion_keeps_a_backedge_into_its_protected_body() {
    let mut ir = sequential_cleanups(1);
    ir.blocks[BlockId::from_index(4)].terminator = Terminator::Goto(BlockId::from_index(1));
    let analysis = Analysis::new(&ir);
    let recovered = crate::structure::cleanup::recover(&analysis, None);
    assert!(recovered.regions.cleanups.consumes(3, 3));
}

#[test]
fn cleanup_continuation_can_test_whether_to_reenter_the_owned_scope() {
    let mut ir = sequential_cleanups(1);
    let header = BlockId::from_index(4);
    let returned = ir.blocks.next_id();
    let zero = ir.values.push(Value {
        local: None,
        kind: ValueKind::Literal(crate::ir::Literal::Int(0)),
        ty: JvmType::Int,
        pc: 4,
        block: header,
    });
    ir.blocks[header].terminator = Terminator::Branch {
        condition: crate::ir::Condition {
            op: crate::ir::CompareOp::Eq,
            left: ir.parameters[0],
            right: zero,
        },
        then_target: returned,
        else_target: BlockId::from_index(1),
    };
    ir.blocks.push(IrBlock {
        caught_exception: None,
        pc: 5,
        statements: vec![],
        terminator: Terminator::Return(None),
        handlers: vec![],
    });
    let analysis = Analysis::new(&ir);
    let recovered = crate::structure::cleanup::recover(&analysis, None);
    assert!(recovered.regions.cleanups.consumes(3, 3));
    assert_eq!(
        crate::structure::tree::walk(&recovered.lower(None).unwrap().body)
            .filter(|statement| matches!(statement, Stmt::TryFinally { .. }))
            .count(),
        1
    );
}

#[test]
fn returning_cleanup_can_be_shared_by_normal_and_exceptional_paths() {
    let mut ir = sequential_cleanups(1);
    let protected = BlockId::from_index(1);
    let normal = BlockId::from_index(2);
    let handler = BlockId::from_index(3);
    let returned = BlockId::from_index(4);
    let shared = ir.blocks.next_id();
    let statements = std::mem::take(&mut ir.blocks[handler].statements);
    ir.blocks.push(IrBlock {
        caught_exception: None,
        pc: shared.index() as u32,
        statements,
        terminator: Terminator::Return(None),
        handlers: vec![],
    });
    ir.blocks[handler].terminator = Terminator::Goto(shared);
    let zero = ir.values.push(Value {
        local: None,
        kind: ValueKind::Literal(crate::ir::Literal::Int(0)),
        ty: JvmType::Int,
        pc: 1,
        block: protected,
    });
    ir.blocks[protected].terminator = Terminator::Branch {
        condition: crate::ir::Condition {
            op: crate::ir::CompareOp::Eq,
            left: ir.parameters[0],
            right: zero,
        },
        then_target: shared,
        else_target: normal,
    };
    let analysis = Analysis::new(&ir);
    let recovered = crate::structure::cleanup::recover(&analysis, None);
    let scope = &recovered.regions.replacements[&protected];
    assert_eq!(
        crate::structure::tree::walk(scope)
            .filter(|statement| matches!(statement, Stmt::TryFinally { .. }))
            .count(),
        1
    );
    assert!(matches!(
        recovered.regions.flow.blocks[handler].end,
        crate::structure::FlowEnd::Unreachable
    ));
    assert!(!recovered.regions.replacements.contains_key(&normal));
    assert!(!recovered.regions.replacements.contains_key(&returned));

    // Without the separate normal copy, this is just code after a swallowed
    // exception. Sharing its continuation does not justify a finally.
    let Terminator::Branch { else_target, .. } = &mut ir.blocks[protected].terminator else {
        panic!()
    };
    *else_target = shared;
    let analysis = Analysis::new(&ir);
    assert!(
        crate::structure::cleanup::recover(&analysis, None)
            .regions
            .replacements
            .is_empty()
    );
    let Terminator::Branch { else_target, .. } = &mut ir.blocks[protected].terminator else {
        panic!()
    };
    *else_target = normal;

    // Joining only the final return skips cleanup. It must not acquire a
    // finally merely because that return belongs to the canonical witness.
    let skipped = ir.blocks.next_id();
    ir.blocks.push(IrBlock {
        caught_exception: None,
        pc: skipped.index() as u32,
        statements: vec![],
        terminator: Terminator::Return(None),
        handlers: vec![],
    });
    ir.blocks[shared].terminator = Terminator::Goto(skipped);
    let Terminator::Branch { then_target, .. } = &mut ir.blocks[protected].terminator else {
        panic!()
    };
    *then_target = skipped;
    let analysis = Analysis::new(&ir);
    assert!(
        crate::structure::cleanup::recover(&analysis, None)
            .regions
            .replacements
            .is_empty()
    );
}

#[test]
fn cleanup_copies_share_a_continuation_without_owning_its_empty_corridors() {
    let mut ir = sequential_cleanups(1);
    let protected = BlockId::from_index(1);
    let normal = BlockId::from_index(2);
    let returned = BlockId::from_index(4);
    let caught = BlockId::from_index(5);
    let copied = BlockId::from_index(6);
    let first_corridor = BlockId::from_index(7);
    let second_corridor = BlockId::from_index(8);
    let zero = ir.values.push(Value {
        local: None,
        kind: ValueKind::Literal(crate::ir::Literal::Int(0)),
        ty: JvmType::Int,
        pc: 0,
        block: ir.entry,
    });
    ir.blocks[ir.entry].terminator = Terminator::Branch {
        condition: crate::ir::Condition {
            op: crate::ir::CompareOp::Eq,
            left: ir.parameters[0],
            right: zero,
        },
        then_target: protected,
        else_target: first_corridor,
    };
    ir.blocks[normal].terminator = Terminator::Goto(first_corridor);
    let exception = ir.values.push(Value {
        local: None,
        kind: ValueKind::CaughtException { handler: caught },
        ty: JvmType::Object("java/lang/Exception".into()),
        pc: 5,
        block: caught,
    });
    let effect = call(&mut ir, caught, "caughtEffect", vec![]);
    ir.blocks.push(IrBlock {
        caught_exception: Some(exception),
        pc: 5,
        statements: vec![Statement::Eval(effect)],
        terminator: Terminator::Goto(copied),
        handlers: ir.blocks[protected].handlers.clone(),
    });
    let parameter = ir.parameters[0];
    let cleanup = call(&mut ir, copied, "cleanup", vec![parameter]);
    ir.blocks.push(IrBlock {
        caught_exception: None,
        pc: 6,
        statements: vec![Statement::Eval(cleanup)],
        terminator: Terminator::Goto(second_corridor),
        handlers: vec![],
    });
    for pc in [7, 8] {
        ir.blocks.push(IrBlock {
            caught_exception: None,
            pc,
            statements: vec![],
            terminator: Terminator::Goto(returned),
            handlers: vec![],
        });
    }
    ir.blocks[protected].handlers.insert(
        0,
        HandlerEdge {
            target: caught,
            catch_type: Some("java/lang/Exception".into()),
            table_index: 1,
        },
    );
    let analysis = Analysis::new(&ir);
    let recovered = crate::structure::cleanup::recover(&analysis, None);
    let regions = recovered.regions;
    assert!(regions.cleanups.consumes(3, 3));
    assert!(
        matches!(regions.flow.blocks[protected].end, crate::structure::FlowEnd::Goto(target) if target == returned)
    );
    assert!(
        regions
            .normal_completions(protected)
            .unwrap()
            .represented_by_edges(&regions.flow.blocks[protected].end)
    );
    for corridor in [first_corridor, second_corridor] {
        assert!(!regions.replacements.contains_key(&corridor));
        assert!(
            matches!(regions.flow.blocks[corridor].end, crate::structure::FlowEnd::Goto(target) if target == returned)
        );
    }
}

#[test]
fn cleanup_continuations_forward_only_returns_stable_across_their_copies() {
    for (writes_returned_local, following_effect) in [(false, false), (true, false), (false, true)]
    {
        let mut ir = sequential_cleanups(1);
        let protected = BlockId::from_index(1);
        let normal = BlockId::from_index(2);
        let handler = BlockId::from_index(3);
        let returned = BlockId::from_index(4);
        ir.return_type = JvmType::Int;
        ir.blocks[returned].terminator = Terminator::Return(Some(ValueId::from_index(1)));
        if writes_returned_local {
            let value = ir.values.push(Value {
                local: None,
                kind: ValueKind::Literal(crate::ir::Literal::Int(7)),
                ty: JvmType::Int,
                pc: 0,
                block: ir.entry,
            });
            for block in [normal, handler] {
                ir.blocks[block].statements.push(Statement::Assign {
                    local: LocalId::from_index(1),
                    value: crate::ir::ValueUse::Reference(value),
                });
            }
        }
        if following_effect {
            let value = call(&mut ir, returned, "afterCleanup", vec![]);
            ir.blocks[returned].statements.push(Statement::Eval(value));
        }
        let analysis = Analysis::new(&ir);
        let mut regions = crate::structure::regions::RegionGraph::new(&ir);
        let candidate = crate::structure::regions::cleanup_candidates(&ir, &regions.flow)
            .into_iter()
            .find(|candidate| candidate.handler == handler)
            .unwrap();
        let plans = plan_candidate(&analysis, &regions, &candidate, None).unwrap();
        for plan in plans {
            regions.install(plan);
        }
        // Keep this a closed atom while enclosing cleanup scopes are proved.
        // Return projection happens only after all graph ownership is final.
        assert!(
            regions
                .normal_completions(protected)
                .unwrap()
                .represented_by_edges(&regions.flow.blocks[protected].end)
        );
        let lowered = regions.lower(None).unwrap();
        let finalized = crate::structure::finalize_source(&ir, lowered).unwrap();
        let body = crate::structure::tree::walk(&finalized.body)
            .find_map(|statement| match statement {
                Stmt::TryFinally { body, .. } => Some(body),
                _ => None,
            })
            .unwrap();
        let forwards = !writes_returned_local && !following_effect;
        assert_eq!(
            crate::structure::tree::walk(body)
                .any(|statement| matches!(statement, Stmt::Return(block) if *block == returned)),
            forwards,
            "writes returned local: {writes_returned_local}, following effect: {following_effect}"
        );
    }
}

#[test]
fn conditional_cleanup_certifies_equivalent_returns_across_handler_ranges() {
    let class = crate::classfile::ClassFile::parse(include_bytes!(
        "../fixtures/exception/classes/pkg/TestFinallyReturnRange.class"
    ))
    .unwrap();
    let method = class
        .methods
        .iter()
        .find(|method| method.name == "run")
        .unwrap();
    let prepared =
        crate::structure::PreparedIr::new(crate::ir::decompile_method(&class, method).unwrap());
    let facts = crate::structure::cleanup_facts(&prepared);
    assert!(facts.consumes(49, 70));
}

#[test]
fn cleanup_body_owns_internal_catches_without_requiring_an_outer_table_suffix() {
    for outer_entries in [0, 2, 3] {
        let mut ir = sequential_cleanups(1);
        let body = BlockId::from_index(1);
        let normal = BlockId::from_index(2);
        let cleanup = BlockId::from_index(3);
        let mut outer = Vec::new();
        for (index, catch_type) in [Some("java/lang/Exception"), None, None]
            .into_iter()
            .enumerate()
        {
            let handler = ir.blocks.next_id();
            let effect = call(&mut ir, handler, "handle", vec![]);
            // Match lifted handler liveness. The last catch-all is shadowed
            // everywhere; the preceding entries receive real caught values.
            let caught_exception = (index < 2).then(|| {
                ir.values.push(Value {
                    local: None,
                    kind: ValueKind::CaughtException { handler },
                    ty: JvmType::Object(catch_type.unwrap_or("java/lang/Throwable").into()),
                    pc: handler.index() as u32,
                    block: handler,
                })
            });
            ir.blocks.push(IrBlock {
                caught_exception,
                pc: handler.index() as u32,
                statements: vec![Statement::Eval(effect)],
                terminator: Terminator::Return(None),
                handlers: vec![],
            });
            outer.push(HandlerEdge {
                target: handler,
                catch_type: catch_type.map(str::to_owned),
                table_index: handler.index(),
            });
        }
        ir.blocks[normal].handlers = outer.clone();
        ir.blocks[cleanup].handlers = outer.clone();
        ir.blocks[body]
            .handlers
            .extend_from_slice(&outer[..outer_entries]);
        let inner = ir.blocks.next_id();
        let caught = ir.values.push(Value {
            local: None,
            kind: ValueKind::CaughtException { handler: inner },
            ty: JvmType::Object("java/lang/Exception".into()),
            pc: inner.index() as u32,
            block: inner,
        });
        ir.blocks.push(IrBlock {
            caught_exception: Some(caught),
            pc: inner.index() as u32,
            statements: vec![],
            terminator: Terminator::Throw {
                value: caught,
                pc: inner.index() as u32,
            },
            handlers: ir.blocks[body].handlers.clone(),
        });
        ir.blocks[body].handlers.insert(
            0,
            HandlerEdge {
                target: inner,
                catch_type: Some("java/lang/Exception".into()),
                table_index: inner.index(),
            },
        );
        let analysis = Analysis::new(&ir);
        let regions = crate::structure::regions::RegionGraph::new(&ir);
        let candidate = crate::structure::regions::cleanup_candidates(&ir, &regions.flow)
            .into_iter()
            .find(|candidate| candidate.handler == cleanup)
            .unwrap();
        let plans = plan_candidate(&analysis, &regions, &candidate, None)
            .unwrap_or_else(|| panic!("outer entries: {outer_entries}"));
        let scope = plans.iter().find(|plan| plan.entry == body).unwrap();
        assert!(scope.cleanups.consumes(3, 3));
        assert_eq!(scope.outer, outer[..2]);
        let catches: Vec<_> = crate::structure::tree::walk(&scope.body)
            .filter_map(|statement| match statement {
                Stmt::Try { catches, .. } => Some(catches),
                _ => None,
            })
            .flatten()
            .map(|catch| catch.handler)
            .collect();
        assert_eq!(catches, [inner], "outer entries: {outer_entries}");
    }
}

#[test]
fn cleanup_equivalence_restores_shadowed_region_bindings() {
    let mut ir = sequential_cleanups(0);
    let head = ir.entry;
    ir.blocks[head].terminator = Terminator::Switch {
        value: ir.parameters[0],
        cases: Vec::new(),
        default: head,
    };
    let analysis = Analysis::new(&ir);
    for kind in 0..3 {
        let cleanup = |outer, inner| {
            let body = vec![Stmt::Break { target: inner }];
            let nested = match kind {
                0 => Stmt::Label {
                    region: inner,
                    body,
                },
                1 => Stmt::Loop {
                    region: inner,
                    header: head,
                    body,
                    continuation: None,
                },
                _ => Stmt::Switch {
                    region: inner,
                    head,
                    cases: vec![crate::structure::SwitchCase {
                        keys: Vec::new(),
                        default: true,
                        body,
                    }],
                },
            };
            vec![Stmt::Label {
                region: outer,
                body: vec![nested, Stmt::Break { target: outer }],
            }]
        };
        let reused = cleanup(7, 7);
        let distinct = cleanup(8, 9);
        assert!(analysis.same(&reused, &distinct), "binder kind {kind}");
        assert!(analysis.same(&distinct, &reused), "binder kind {kind}");
    }
}

#[test]
fn cleanup_equivalence_restores_shadowed_loop_headers() {
    let ir = sequential_cleanups(0);
    let analysis = Analysis::new(&ir);
    let cleanup = |region, outer, inner| {
        vec![Stmt::Loop {
            region,
            header: outer,
            continuation: None,
            body: vec![
                Stmt::Loop {
                    region,
                    header: inner,
                    continuation: None,
                    body: vec![Stmt::Break { target: region }],
                },
                Stmt::Continue { header: outer },
            ],
        }]
    };
    let reused = cleanup(7, BlockId::from_index(0), BlockId::from_index(0));
    let distinct = cleanup(9, BlockId::from_index(1), BlockId::from_index(0));
    assert!(analysis.same(&reused, &distinct));
    assert!(analysis.same(&distinct, &reused));
}

#[test]
fn cleanup_equivalence_rejects_transfers_captured_by_shadowing() {
    let ir = sequential_cleanups(0);
    let analysis = Analysis::new(&ir);
    for is_continue in [false, true] {
        let cleanup = |inner_region, inner_header| {
            vec![Stmt::Loop {
                region: 7,
                header: BlockId::from_index(0),
                continuation: None,
                body: vec![Stmt::Loop {
                    region: inner_region,
                    header: inner_header,
                    continuation: None,
                    body: vec![if is_continue {
                        Stmt::Continue {
                            header: BlockId::from_index(0),
                        }
                    } else {
                        Stmt::Break { target: 7 }
                    }],
                }],
            }]
        };
        let outward = cleanup(8, BlockId::from_index(1));
        let captured = cleanup(7, BlockId::from_index(0));
        assert!(!analysis.same(&outward, &captured));
        assert!(!analysis.same(&captured, &outward));
    }
}

#[test]
fn cleanup_writes_include_owned_operations_and_exact_fragments() {
    let mut ir = sequential_cleanups(1);
    let source = ir.parameters[0];
    let inline = |ir: &mut MethodIr| {
        let local = ir.locals.push(Local {
            name_hint: None,
            name: "written".into(),
            ty: JvmType::Int,
            parameter: None,
            is_this: false,
            needs_default: false,
        });
        let value = ir.values.push(Value {
            local: None,
            kind: ValueKind::InlineAssign {
                local,
                value: source,
            },
            ty: JvmType::Int,
            pc: 0,
            block: ir.entry,
        });
        (local, value)
    };
    let (fragment_local, fragment) = inline(&mut ir);
    let (update_local, update) = inline(&mut ir);
    let (monitor_local, monitor) = inline(&mut ir);
    let (predicate_local, predicate) = inline(&mut ir);
    let entry = ir.entry;
    ir.blocks[entry].statements.push(Statement::Eval(fragment));
    let continuation = BlockId::from_index(4);
    ir.blocks[continuation]
        .statements
        .push(Statement::Eval(update));
    let condition = BlockId::from_index(1);
    ir.blocks[condition].terminator = Terminator::Branch {
        condition: crate::ir::Condition {
            op: crate::ir::CompareOp::Eq,
            left: predicate,
            right: source,
        },
        then_target: continuation,
        else_target: continuation,
    };
    let handler = BlockId::from_index(3);
    let caught = ir.values[ir.blocks[handler].caught_exception.unwrap()]
        .local
        .unwrap();
    let cleanup = vec![Stmt::Try {
        body: vec![Stmt::Loop {
            region: 0,
            header: entry,
            continuation: Some(continuation),
            body: vec![
                Stmt::Statements {
                    block: entry,
                    start: 1,
                    end: 2,
                },
                Stmt::Synchronized {
                    monitor,
                    body: Vec::new(),
                },
                Stmt::If {
                    condition: Predicate::Atom {
                        head: condition,
                        negated: false,
                    },
                    then_body: Vec::new(),
                    else_body: Vec::new(),
                },
            ],
        }],
        catches: vec![crate::structure::Catch {
            catch_type: None,
            handler,
            body: Vec::new(),
        }],
    }];
    assert_eq!(
        equivalent::written_locals(&ir, &cleanup),
        BTreeSet::from([
            fragment_local,
            update_local,
            monitor_local,
            predicate_local,
            caught
        ]),
    );
}
