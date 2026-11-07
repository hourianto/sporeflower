use super::*;
use crate::cfg::HandlerEdge;
use crate::classfile::JvmType;
use crate::index::{LocalId, ValueId};
use crate::ir::{InvokeKind, IrBlock, Local, Member, Statement, Value, ValueKind};
use crate::structure::Predicate;

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
        let tree =
            crate::structure::finalize_source(&analysis, recovered.lower(None).unwrap()).unwrap();
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
        for catch_type in [Some("java/lang/Exception"), None, None] {
            let handler = ir.blocks.next_id();
            let effect = call(&mut ir, handler, "handle", vec![]);
            ir.blocks.push(IrBlock {
                caught_exception: None,
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
