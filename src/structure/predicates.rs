//! Project source predicates and continuations after cleanup ownership is final.
//! One scoped traversal normalizes each child once. Scheduled condition effects
//! retain their evaluation point; pure exit stores follow continuation proofs.
#[cfg(test)]
use super::continuations::redirect_exits;
use super::continuations::{Bindings, Continuation, ImmediateExit, Scope};
use super::{Predicate, Stmt};
use crate::ir::MethodIr;

#[cfg(test)]
mod loop_tests;

pub(super) fn normalize(ir: &MethodIr, body: Vec<Stmt>) -> Vec<Stmt> {
    normalize_with_exit(ir, body, None)
}

// Own the pending traversal explicitly: large switch/handler ladders can have
// thousands of lexical regions even when their bytecode graph is acyclic.
// Child bodies are normalized once, then moved back into their parent.
enum Work {
    Sequence(Vec<Stmt>, Option<Continuation>, Scope),
    FinishSequence {
        remaining: Vec<Stmt>,
        fragments: Vec<Vec<Stmt>>,
        scope: Scope,
        exit: Option<Continuation>,
    },
    Statement(Stmt, Option<Continuation>, Scope),
    FinishStatement(Stmt),
}

fn normalize_with_exit(
    ir: &MethodIr,
    body: Vec<Stmt>,
    implicit_exit: Option<Continuation>,
) -> Vec<Stmt> {
    let mut work = vec![Work::Sequence(body, implicit_exit, None)];
    let mut bindings = Bindings::default();
    let mut completed: Vec<Vec<Stmt>> = Vec::new();
    while let Some(task) = work.pop() {
        match task {
            Work::Sequence(mut body, exit, scope) => {
                if let Some(statement) = body.pop() {
                    work.push(Work::FinishSequence {
                        remaining: body,
                        fragments: Vec::new(),
                        scope,
                        exit: exit.clone(),
                    });
                    work.push(Work::Statement(statement, exit, scope));
                } else {
                    completed.push(Vec::new());
                }
            }
            Work::Statement(mut statement, exit, scope) => {
                if let Stmt::Break { target } = statement {
                    completed.push(bindings.transfer(ir, target, scope));
                    continue;
                }
                // Record the lexical owner once. Transfers resolve through
                // that environment when visited, without rewriting descendants.
                let scope = statement.binder().map_or(scope, |binder| {
                    let follow = match statement {
                        Stmt::Label { .. } => exit.clone(),
                        Stmt::Switch { .. } => exit.clone().filter(|exit| exit.direct_return(ir)),
                        Stmt::Loop {
                            region, ref body, ..
                        } if head_exit(ir, body, region) => {
                            exit.clone().filter(|exit| exit.direct_return(ir))
                        }
                        _ => None,
                    };
                    bindings.bind(scope, binder.region(), follow)
                });
                let child_exit = match &statement {
                    Stmt::If { .. } => exit,
                    Stmt::Label { region, .. } => {
                        exit.or_else(|| Some(Continuation::from(ImmediateExit::Break(*region))))
                    }
                    Stmt::Loop { header, .. } => Some(ImmediateExit::Continue(*header).into()),
                    _ => None,
                };
                let scopes = bindings.children(ir, &statement, scope);
                let bodies: Vec<_> = statement.bodies_mut().map(std::mem::take).collect();
                if bodies.is_empty() {
                    completed.push(normalize_statement(ir, statement));
                } else {
                    work.push(Work::FinishStatement(statement));
                    for (body, scope) in bodies.into_iter().zip(scopes).rev() {
                        work.push(Work::Sequence(body, child_exit.clone(), scope));
                    }
                }
            }
            Work::FinishStatement(mut statement) => {
                let count = statement.bodies().count();
                let normalized = completed.split_off(completed.len() - count);
                for (body, normalized) in statement.bodies_mut().zip(normalized) {
                    *body = normalized;
                }
                completed.push(normalize_statement(ir, statement));
            }
            Work::FinishSequence {
                mut remaining,
                mut fragments,
                scope,
                exit,
            } => {
                let normalized = completed.pop().unwrap();
                // A successor label can normalize to a terminal transfer.
                // Propagate that completed result to its predecessor, rather
                // than precomputing continuations from the unnormalized tree.
                let exit = normalized.iter().rev().fold(exit, |exit, statement| {
                    Continuation::before(ir, statement, exit)
                });
                fragments.push(normalized);
                if let Some(statement) = remaining.pop() {
                    work.push(Work::FinishSequence {
                        remaining,
                        fragments,
                        scope,
                        exit: exit.clone(),
                    });
                    work.push(Work::Statement(statement, exit, scope));
                    continue;
                }
                let mut result = Vec::new();
                for statements in fragments.into_iter().rev() {
                    // Redirecting an inner label can make its parent abrupt.
                    // Its original trailing transfer is then unreachable.
                    if !super::can_complete(&result) {
                        break;
                    }
                    append(&mut result, statements);
                }
                completed.push(result);
            }
        }
    }
    debug_assert_eq!(completed.len(), 1);
    completed.pop().unwrap()
}

fn empty(ir: &MethodIr, statement: &Stmt) -> bool {
    statement
        .scheduled(ir)
        .is_some_and(|scheduled| scheduled.statements.is_empty())
}

fn head_exit(ir: &MethodIr, body: &[Stmt], region: usize) -> bool {
    let Some(Stmt::If {
        then_body,
        else_body,
        ..
    }) = body.iter().find(|statement| !empty(ir, statement))
    else {
        return false;
    };
    let leaves =
        |body: &[Stmt]| matches!(body.last(), Some(Stmt::Break { target }) if *target == region);
    // A leading normal exit can become the loop bound after result projection.
    // Tail exits keep their loop owner so do/while and preupdated tests retain
    // their existing iteration boundary instead of becoming infinite loops.
    leaves(then_body) || leaves(else_body)
}

fn normalize_statement(ir: &MethodIr, statement: Stmt) -> Vec<Stmt> {
    match statement {
        Stmt::Basic(_) | Stmt::Statements { .. } if empty(ir, &statement) => Vec::new(),
        Stmt::If {
            condition,
            mut then_body,
            mut else_body,
        } => {
            let mut continuation = shared_stores(ir, &mut then_body, &mut else_body);
            continuation.insert(0, fold_if(condition, then_body, else_body));
            continuation
        }
        Stmt::Label { region, body } => super::stackify::simplify_label(region, body),
        Stmt::Loop {
            region,
            header,
            continuation,
            mut body,
        } => {
            // Falling through the loop body already takes its own final jump.
            if matches!(body.last(), Some(Stmt::Continue { header: target }) if *target == header) {
                body.pop();
            }
            let outward = loop_guard(&mut body, region, header);
            let mut result = vec![Stmt::Loop {
                region,
                header,
                continuation,
                body,
            }];
            result.extend(outward);
            result
        }
        other => vec![other],
    }
}

/// Phi edges can repeat one local store at every leaf of a source decision.
/// Its identical scheduled suffix belongs to the decision's continuation.
/// Only direct stores move: an abrupt exit or protected child keeps its owner.
fn shared_stores(ir: &MethodIr, then_body: &mut Vec<Stmt>, else_body: &mut Vec<Stmt>) -> Vec<Stmt> {
    let mut tail = Vec::new();
    while let (Some(left), Some(right)) = (
        then_body
            .last()
            .and_then(|statement| statement.scheduled(ir)),
        else_body
            .last()
            .and_then(|statement| statement.scheduled(ir)),
    ) {
        let Some(crate::ir::Statement::Assign { local, value }) = left.statements.last() else {
            break;
        };
        if !matches!(right.statements.last(),
            Some(crate::ir::Statement::Assign { local: other, value: source })
                if other == local && source == value)
        {
            break;
        }
        tail.push(Stmt::Statements {
            block: left.block,
            start: left.range().end - 1,
            end: left.range().end,
        });
        for (body, scheduled) in [(&mut *then_body, left), (&mut *else_body, right)] {
            let end = scheduled.range().end - 1;
            if end == scheduled.start {
                body.pop();
            } else {
                *body.last_mut().unwrap() = Stmt::Statements {
                    block: scheduled.block,
                    start: scheduled.start,
                    end,
                };
            }
        }
    }
    tail.reverse();
    tail
}

/// Give a loop's single guarded normal exit to the loop itself. The original
/// outward transfer stays immediately after it, under the same handler and
/// monitor parents. Both transfers skip an owned iteration update, and no
/// return value is evaluated until the identical exit has actually been taken.
fn loop_guard(
    body: &mut Vec<Stmt>,
    region: super::RegionId,
    header: super::BlockId,
) -> Option<Stmt> {
    // An empty loop can encode its test as an explicit backedge followed by
    // its exit. Make that same edge the implicit end of the loop body.
    if let [
        Stmt::If {
            condition,
            then_body,
            else_body,
        },
        Stmt::Break { target },
    ] = body.as_slice()
        && *target == region
        && else_body.is_empty()
        && matches!(then_body.as_slice(), [Stmt::Continue { header: target }] if *target == header)
    {
        *body = vec![Stmt::If {
            condition: condition.clone().negated(),
            then_body: vec![Stmt::Break { target: region }],
            else_body: Vec::new(),
        }];
        return None;
    }
    let [
        Stmt::If {
            then_body,
            else_body,
            ..
        },
        rest @ ..,
    ] = body.as_mut_slice()
    else {
        return None;
    };
    let [transfer] = then_body.as_mut_slice() else {
        return None;
    };
    let outward = match transfer {
        Stmt::Return(_) => true,
        Stmt::Break { target } => *target != region,
        Stmt::Continue { header: target } => *target != header,
        _ => false,
    };
    if !outward || !else_body.is_empty() || super::contains_break(rest, region) {
        return None;
    }
    // Direct scope is important: the loop is the only binder being removed
    // from this transfer's ancestry. A different target retains its owner,
    // even while independently recovered inner regions still reuse IDs.
    Some(std::mem::replace(transfer, Stmt::Break { target: region }))
}

fn append(result: &mut Vec<Stmt>, statements: Vec<Stmt>) {
    for statement in statements {
        let Stmt::If {
            mut condition,
            mut then_body,
            mut else_body,
        } = statement
        else {
            result.push(statement);
            continue;
        };
        if let Some(Stmt::If {
            condition: previous,
            then_body: prior_then,
            else_body: prior_else,
        }) = result.last_mut()
            && prior_else.is_empty()
            && else_body.is_empty()
            && same_exit(prior_then, &then_body)
        {
            // An abrupt first arm prevents the second test from being
            // evaluated. It is exactly the short-circuit OR rule.
            let left = std::mem::replace(previous, Predicate::And(Vec::new()));
            *previous = left.combine(condition, false);
            continue;
        }
        // Keep an abrupt arm as the guard. With two abrupt arms,
        // prefer the direct exit and expose the nested decisions.
        if !super::can_complete(&else_body)
            && (super::can_complete(&then_body)
                || direct_exit(&else_body) && !direct_exit(&then_body))
        {
            condition = condition.negated();
            std::mem::swap(&mut then_body, &mut else_body);
        }
        let continuation = if super::can_complete(&then_body) {
            Vec::new()
        } else {
            std::mem::take(&mut else_body)
        };
        result.push(Stmt::If {
            condition,
            then_body,
            else_body,
        });
        result.extend(continuation);
    }
}

fn direct_exit(body: &[Stmt]) -> bool {
    let Some((exit, prefix)) = body.split_last() else {
        return false;
    };
    matches!(
        exit,
        Stmt::Return(_) | Stmt::Throw(_) | Stmt::Break { .. } | Stmt::Continue { .. }
    ) && prefix
        .iter()
        .all(|statement| matches!(statement, Stmt::Basic(_) | Stmt::Statements { .. }))
}

pub(super) fn fold_if(
    mut condition: Predicate,
    mut then_body: Vec<Stmt>,
    mut else_body: Vec<Stmt>,
) -> Stmt {
    if then_body.is_empty() && !else_body.is_empty() {
        condition = condition.negated();
        std::mem::swap(&mut then_body, &mut else_body);
    }
    // Conditional bytecodes commonly jump over the matching arm to the next
    // test. Keep that decision chain in `else`, where Java can express it at
    // one indentation level, rather than nesting the whole chain in `then`.
    if matches!(then_body.as_slice(), [Stmt::If { .. }])
        && !else_body.is_empty()
        && !matches!(else_body.as_slice(), [Stmt::If { .. }])
    {
        condition = condition.negated();
        std::mem::swap(&mut then_body, &mut else_body);
    }
    while else_body.is_empty()
        && matches!(then_body.as_slice(), [Stmt::If { else_body, .. }] if else_body.is_empty())
    {
        let Stmt::If {
            condition: next,
            then_body: body,
            ..
        } = then_body.pop().unwrap()
        else {
            unreachable!()
        };
        condition = condition.combine(next, true);
        then_body = body;
    }
    Stmt::If {
        condition,
        then_body,
        else_body,
    }
}

fn same_exit(left: &[Stmt], right: &[Stmt]) -> bool {
    match (left, right) {
        ([Stmt::Break { target: a }], [Stmt::Break { target: b }]) => a == b,
        ([Stmt::Continue { header: a }], [Stmt::Continue { header: b }]) => a == b,
        (
            [
                Stmt::DispatchContinue {
                    region: a,
                    target: x,
                },
            ],
            [
                Stmt::DispatchContinue {
                    region: b,
                    target: y,
                },
            ],
        ) => a == b && x == y,
        ([Stmt::Return(a)], [Stmt::Return(b)])
        | ([Stmt::Throw(a)], [Stmt::Throw(b)])
        | ([Stmt::Jump(a)], [Stmt::Jump(b)]) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    const B0: crate::index::BlockId = crate::index::BlockId::from_index(0);
    const B1: crate::index::BlockId = crate::index::BlockId::from_index(1);
    const B2: crate::index::BlockId = crate::index::BlockId::from_index(2);
    const B4: crate::index::BlockId = crate::index::BlockId::from_index(4);
    const B5: crate::index::BlockId = crate::index::BlockId::from_index(5);
    const B6: crate::index::BlockId = crate::index::BlockId::from_index(6);
    const B10: crate::index::BlockId = crate::index::BlockId::from_index(10);
    const B12: crate::index::BlockId = crate::index::BlockId::from_index(12);
    const L0: crate::index::LocalId = crate::index::LocalId::from_index(0);
    const L1: crate::index::LocalId = crate::index::LocalId::from_index(1);
    const V0: crate::index::ValueId = crate::index::ValueId::from_index(0);
    const V1: crate::index::ValueId = crate::index::ValueId::from_index(1);
    const V5: crate::index::ValueId = crate::index::ValueId::from_index(5);
    use super::*;
    use crate::classfile::JvmType;
    use crate::ir::{BinaryOp, IrBlock, Literal, Terminator, Value, ValueKind};

    fn empty_ir() -> MethodIr {
        MethodIr {
            entry: B0,
            values: vec![].into(),
            blocks: vec![].into(),
            locals: vec![].into(),

            parameters: vec![],
            return_type: JvmType::Void,

            initialization_sites: vec![],
        }
    }

    fn returning_ir() -> MethodIr {
        let mut ir = empty_ir();
        ir.values = [
            ValueKind::Literal(Literal::Int(17)),
            ValueKind::Parameter {
                slot: 0,
                index: Some(0),
            },
            ValueKind::Binary {
                op: BinaryOp::Add,
                left: V0,
                right: V1,
            },
            ValueKind::Binary {
                op: BinaryOp::Div,
                left: V0,
                right: V1,
            },
            ValueKind::InlineAssign {
                local: L0,
                value: V0,
            },
        ]
        .into_iter()
        .map(|kind| Value {
            local: None,
            kind,
            ty: JvmType::Int,
            pc: 0,
            block: B0,
        })
        .collect();
        ir.values[crate::index::ValueId::from_index(1)].local =
            Some(crate::index::LocalId::from_index(0));
        ir.locals.push(crate::ir::Local {
            name_hint: None,
            name: "result".into(),
            ty: JvmType::Int,
            parameter: Some(0),
            is_this: false,
            needs_default: false,
        });
        ir.blocks.push(IrBlock {
            caught_exception: None,
            pc: 0,
            statements: Vec::new(),
            terminator: Terminator::Return(Some(V0)),
            handlers: Vec::new(),
        });
        ir.blocks.push(IrBlock {
            caught_exception: None,
            pc: 1,
            statements: vec![crate::ir::Statement::Assign {
                local: L0,
                value: crate::ir::ValueUse::Reference(V0),
            }],
            terminator: Terminator::Return(None),
            handlers: Vec::new(),
        });
        ir
    }

    #[test]
    fn return_continuations_bound_expansion_and_reject_effects_or_throwing_operands() {
        let mut ir = returning_ir();
        for (value, eligible, constant) in [
            (0, true, true),
            (1, true, false),
            (2, true, false),
            (3, false, false),
            (4, false, false),
        ] {
            ir.blocks[B0].terminator =
                Terminator::Return(Some(crate::index::ValueId::from_index(value)));
            assert_eq!(ImmediateExit::return_expression(&ir, B0, false), eligible);
            assert_eq!(ImmediateExit::return_expression(&ir, B0, true), constant);
        }
        let mut operand = crate::index::ValueId::from_index(0);
        for _ in 0..5 {
            let value = ir.values.next_id();
            ir.values.push(Value {
                local: None,
                kind: ValueKind::Binary {
                    op: BinaryOp::Add,
                    left: operand,
                    right: operand,
                },
                ty: JvmType::Int,
                pc: 0,
                block: B0,
            });

            operand = value;
        }
        ir.blocks[B0].terminator = Terminator::Return(Some(operand));
        assert!(!ImmediateExit::return_expression(&ir, B0, false));
    }

    #[test]
    fn return_continuations_respect_cleanup_and_shadowed_label_boundaries() {
        let mut ir = returning_ir();
        let original = vec![
            Stmt::TryFinally {
                body: vec![Stmt::Synchronized {
                    monitor: V0,
                    body: vec![Stmt::Break { target: 9 }],
                }],
                cleanup: vec![Stmt::Basic(B1)],
                handler: B2,
            },
            Stmt::Label {
                region: 9,
                body: vec![Stmt::Break { target: 9 }],
            },
        ];
        let mut literal = original.clone();
        redirect_exits(&ir, &mut literal, 9, &ImmediateExit::Return(B0).into());
        assert!(
            super::super::tree::walk(&literal)
                .any(|statement| matches!(statement, Stmt::Return(B0)))
        );
        assert!(
            matches!(&literal[1], Stmt::Label { body, .. } if matches!(body.as_slice(), [Stmt::Break { target: 9 }]))
        );
        ir.blocks[B0].terminator = Terminator::Return(Some(V1));
        let mut dependent = original.clone();
        redirect_exits(&ir, &mut dependent, 9, &ImmediateExit::Return(B0).into());
        assert_eq!(format!("{dependent:?}"), format!("{original:?}"));
        ir.blocks[B1].statements.clear();
        redirect_exits(&ir, &mut dependent, 9, &ImmediateExit::Return(B0).into());
        assert!(
            super::super::tree::walk(&dependent)
                .any(|statement| matches!(statement, Stmt::Return(B0)))
        );
    }

    #[test]
    fn nested_terminal_regions_bind_before_child_recovery() {
        let mut ir = returning_ir();
        ir.values.push(Value {
            local: None,
            kind: ValueKind::Literal(Literal::Int(31)),
            ty: JvmType::Int,
            pc: 2,
            block: B2,
        });

        ir.blocks.push(IrBlock {
            caught_exception: None,
            pc: 2,
            statements: Vec::new(),
            terminator: Terminator::Return(Some(V5)),
            handlers: Vec::new(),
        });
        let body = vec![
            Stmt::Label {
                region: 0,
                body: vec![
                    Stmt::Label {
                        region: 1,
                        body: vec![Stmt::TryFinally {
                            body: vec![
                                Stmt::Label {
                                    region: 2,
                                    body: vec![
                                        Stmt::Label {
                                            region: 3,
                                            body: vec![Stmt::If {
                                                condition: Predicate::Atom {
                                                    head: B0,
                                                    negated: false,
                                                },
                                                then_body: vec![Stmt::Break { target: 3 }],
                                                else_body: vec![Stmt::Break { target: 2 }],
                                            }],
                                        },
                                        Stmt::Break { target: 1 },
                                    ],
                                },
                                Stmt::Break { target: 0 },
                            ],
                            cleanup: vec![Stmt::Basic(B1)],
                            handler: B0,
                        }],
                    },
                    Stmt::Return(B0),
                ],
            },
            Stmt::Return(B2),
        ];
        let body = normalize(&ir, body);
        assert_eq!(
            crate::structure::statistics(&body).labeled_regions,
            0,
            "{body:#?}"
        );
        for expected in [0, 2] {
            assert!(
                super::super::tree::walk(&body).any(
                    |statement| matches!(statement, Stmt::Return(block) if block.index() == expected)
                ),
                "{body:#?}"
            );
        }
    }

    #[test]
    fn normalized_successors_supply_the_previous_regions_continuation() {
        let ir = returning_ir();
        let body = vec![
            Stmt::Label {
                region: 0,
                body: vec![Stmt::Loop {
                    region: 1,
                    header: B0,
                    continuation: None,
                    body: vec![Stmt::If {
                        condition: Predicate::Atom {
                            head: B0,
                            negated: false,
                        },
                        then_body: vec![Stmt::Break { target: 0 }],
                        else_body: vec![Stmt::Break { target: 1 }],
                    }],
                }],
            },
            Stmt::Label {
                region: 2,
                body: vec![Stmt::Break { target: 2 }],
            },
            Stmt::Return(B0),
        ];
        let body = normalize(&ir, body);
        assert_eq!(
            crate::structure::statistics(&body).labeled_regions,
            0,
            "{body:#?}"
        );
    }

    #[test]
    fn deep_forward_regions_do_not_use_recursive_normalization() {
        let mut body = vec![Stmt::Continue { header: B0 }];
        for region in 0..8192 {
            body = vec![Stmt::Label { region, body }];
        }
        assert!(matches!(
            normalize(&empty_ir(), body).as_slice(),
            [Stmt::Continue { header: B0 }]
        ));
    }

    #[test]
    fn deep_bound_forward_regions_resolve_at_the_transfer_once() {
        let ir = returning_ir();
        let mut body = vec![Stmt::Break { target: 8191 }];
        for region in 0..8192 {
            body = vec![Stmt::Label { region, body }];
        }
        body.push(Stmt::Return(B0));
        assert!(matches!(
            normalize(&ir, body).as_slice(),
            [Stmt::Return(B0)]
        ));
    }

    #[test]
    fn result_stores_keep_cleanup_and_enclosing_iteration_ownership() {
        let mut ir = returning_ir();
        ir.locals.push(crate::ir::Local {
            name_hint: None,
            name: "observed".into(),
            ty: JvmType::Int,
            parameter: None,
            is_this: false,
            needs_default: false,
        });
        ir.blocks.push(IrBlock {
            caught_exception: None,
            pc: 2,
            statements: vec![crate::ir::Statement::Assign {
                local: L1,
                value: crate::ir::ValueUse::Reference(V1),
            }],
            terminator: Terminator::Return(None),
            handlers: Vec::new(),
        });
        let continuation =
            Continuation::before(&ir, &Stmt::Basic(B1), Some(ImmediateExit::Break(9).into()))
                .unwrap();
        let mut bindings = Bindings::default();
        let scope = bindings.bind(None, 7, Some(continuation));
        for cleanup in [
            vec![Stmt::Basic(B1)],
            vec![Stmt::Basic(B2)],
            vec![Stmt::Continue { header: B12 }],
            vec![Stmt::Break { target: 11 }],
        ] {
            let statement = Stmt::TryFinally {
                body: Vec::new(),
                cleanup,
                handler: B0,
            };
            let scope = bindings.children(&ir, &statement, scope)[0];
            assert!(matches!(
                bindings.transfer(&ir, 7, scope).as_slice(),
                [Stmt::Break { target: 7 }]
            ));
        }
        let statement = Stmt::TryFinally {
            body: Vec::new(),
            cleanup: Vec::new(),
            handler: B0,
        };
        let inner = bindings.children(&ir, &statement, scope)[0];
        assert!(matches!(
            bindings.transfer(&ir, 7, inner).as_slice(),
            [Stmt::Break { target: 7 }]
        ));
        let statement = Stmt::Synchronized {
            monitor: V0,
            body: Vec::new(),
        };
        let inner = bindings.children(&ir, &statement, scope)[0];
        assert!(matches!(
            bindings.transfer(&ir, 7, inner).as_slice(),
            [Stmt::Break { target: 7 }]
        ));
        let statement = Stmt::Loop {
            region: 3,
            header: B4,
            continuation: None,
            body: Vec::new(),
        };
        let inner = bindings.children(&ir, &statement, scope)[0];
        assert!(matches!(
            bindings.transfer(&ir, 7, inner).as_slice(),
            [Stmt::Basic(B1), Stmt::Break { target: 9 }]
        ));
        assert!(
            Continuation::before(
                &ir,
                &Stmt::Basic(B1),
                Some(ImmediateExit::Continue(B10).into()),
            )
            .is_none()
        );
    }

    #[test]
    fn iteration_tail_labels_bind_the_enclosing_header() {
        let ir = empty_ir();
        let tail = Stmt::Label {
            region: 2,
            body: vec![Stmt::Loop {
                region: 3,
                header: B4,
                continuation: None,
                body: vec![Stmt::If {
                    condition: Predicate::Atom {
                        head: B5,
                        negated: false,
                    },
                    then_body: vec![Stmt::Break { target: 2 }],
                    else_body: vec![Stmt::Break { target: 3 }],
                }],
            }],
        };
        let body = normalize(
            &ir,
            vec![Stmt::Loop {
                region: 0,
                header: B1,
                continuation: Some(B6),
                body: vec![tail],
            }],
        );
        assert_eq!(crate::structure::statistics(&body).labeled_regions, 0);
        let Stmt::Loop {
            body,
            continuation: Some(B6),
            ..
        } = &body[0]
        else {
            panic!("{body:#?}")
        };
        let Stmt::Loop {
            body, header: B4, ..
        } = &body[0]
        else {
            panic!("{body:#?}")
        };
        assert!(
            matches!(body.last(), Some(Stmt::Break { target: 3 })),
            "{body:#?}"
        );
        assert!(matches!(
            &body[0], Stmt::If { then_body, .. }
                if matches!(then_body.as_slice(), [Stmt::Continue { header: B1 }])
        ));
    }
}
