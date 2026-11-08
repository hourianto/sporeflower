//! Preparation reuses scheduled IR local facts before expressions are bound.
use super::Expression;
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
