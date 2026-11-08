//! Access provenance in the final source plan reuses scheduled IR local facts.
use super::{Body, Expression, LoopHeader, Selector, Simple, Statement, Throw};
use crate::ir::{LocalId, MethodIr, ValueUse};
use std::collections::BTreeSet;

pub(in crate::java) fn expression(ir: &MethodIr, expression: &Expression) -> BTreeSet<LocalId> {
    let mut result = BTreeSet::new();
    let mut updates = BTreeSet::new();
    for_each_value(
        ir,
        expression,
        |usage| result.extend(crate::java::local_flow::value_accesses(ir, usage)),
        |local| {
            updates.insert(local);
        },
    );
    result.extend(updates);
    result
}

/// Scheduled leaves and explicit source updates share one traversal for local
/// access, mutation ownership, and exception-effect queries.
pub(super) fn for_each_value(
    ir: &MethodIr,
    expression: &Expression,
    mut value: impl FnMut(ValueUse),
    mut update: impl FnMut(LocalId),
) {
    let mut pending = vec![expression];
    while let Some(expression) = pending.pop() {
        match expression {
            Expression::Value(usage) => value(*usage),
            Expression::Update { local, .. } => {
                update(*local);
            }
            Expression::Condition(condition) => {
                for operand in [condition.left, condition.right] {
                    value(ValueUse::Reference(operand));
                }
            }
            Expression::Predicate { predicate, .. } => {
                for head in predicate.heads() {
                    for operand in ir.blocks[head].terminator.operands() {
                        value(ValueUse::Reference(operand));
                    }
                }
            }
            Expression::Select { condition, yes, no } => {
                pending.extend([condition.as_ref(), yes.as_ref(), no.as_ref()]);
            }
            Expression::Compare { left, right, .. } => {
                pending.extend([left.as_ref(), right.as_ref()])
            }
            Expression::Boolean(_) => (),
        }
    }
}

pub(in crate::java) fn simple(ir: &MethodIr, simple: &Simple) -> BTreeSet<LocalId> {
    match simple {
        Simple::Declare { local, .. } => BTreeSet::from([*local]),
        Simple::Assign { local, value, .. } => {
            let mut result = expression(ir, value);
            result.insert(*local);
            result
        }
        Simple::Scheduled(statement) => crate::java::local_flow::statement_accesses(ir, statement),
        Simple::CatchBind { local, .. } => BTreeSet::from([*local]),
        Simple::Dispatch { .. } => BTreeSet::new(),
    }
}

pub(in crate::java) fn body(ir: &MethodIr, body: &Body) -> BTreeSet<LocalId> {
    let mut result = BTreeSet::new();
    let mut pending = vec![body];
    while let Some(body) = pending.pop() {
        for statement in body {
            match statement {
                Statement::Simple(value) => result.extend(simple(ir, value)),
                Statement::Return(Some(value)) => result.extend(expression(ir, value)),
                Statement::Throw(value) => result.extend(throwing(ir, value)),
                Statement::If { condition, yes, no } => {
                    result.extend(expression(ir, condition));
                    pending.extend([yes, no]);
                }
                Statement::Loop { header, body, .. } => {
                    match header {
                        LoopHeader::While(value) | LoopHeader::DoWhile(value) => {
                            result.extend(expression(ir, value))
                        }
                        LoopHeader::For {
                            initializer,
                            condition,
                            update,
                        } => {
                            if let Some(initializer) = initializer {
                                result.extend(simple(ir, initializer));
                            }
                            result.extend(expression(ir, condition));
                            result.extend(simple(ir, update));
                        }
                    }
                    pending.push(body);
                }
                Statement::Switch { value, cases, .. } => {
                    if let Selector::Value(value) = value {
                        result.extend(crate::java::local_flow::value_accesses(
                            ir,
                            ValueUse::Reference(*value),
                        ));
                    }
                    pending.extend(cases.iter().map(|case| &case.body));
                }
                Statement::Synchronized { monitor, body } => {
                    result.extend(expression(ir, monitor));
                    pending.push(body);
                }
                Statement::Block { body, .. } => pending.push(body),
                Statement::Try {
                    body,
                    catches,
                    cleanup,
                } => {
                    pending.push(body);
                    pending.extend(catches.iter().map(|catch| &catch.body));
                    if let Some(cleanup) = cleanup {
                        pending.push(cleanup);
                    }
                }
                _ => (),
            }
        }
    }
    result
}

pub(in crate::java) fn throwing(ir: &MethodIr, throwing: &Throw) -> BTreeSet<LocalId> {
    match throwing {
        Throw::Value { value, .. } => {
            crate::java::local_flow::value_accesses(ir, ValueUse::Reference(*value))
        }
        Throw::Witness(_) | Throw::InvalidDispatch => BTreeSet::new(),
    }
}
