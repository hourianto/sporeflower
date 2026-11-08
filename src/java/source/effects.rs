//! Java's checked-catch and method-boundary rules apply to the selected source
//! operations. Handler ownership was proved earlier; this pass never changes it.
use super::{Body, Expression, LoopHeader, Selector, Simple, Statement, Throw};
use crate::ir::scheduled::ValueUse;
use crate::java::{
    exceptions::{self, Thrown},
    expr::ExprPlanner,
};

pub(super) fn finalize(expr: &ExprPlanner<'_, '_>, body: &mut Body) -> Thrown {
    let mut result = Thrown::default();
    for statement in body {
        match statement {
            Statement::Simple(value) => simple(expr, value, &mut result),
            Statement::Return(Some(value)) => expression(expr, value, &mut result),
            Statement::Throw(Throw::Value { value, pc }) => {
                value_effects(expr, ValueUse::Reference(*value), &mut result);
                exceptions::throw_effects(
                    expr.cx.program,
                    expr.member,
                    expr.ir,
                    *value,
                    *pc,
                    &mut result,
                );
            }
            Statement::If { condition, yes, no } => {
                expression(expr, condition, &mut result);
                result.extend(finalize(expr, yes));
                result.extend(finalize(expr, no));
            }
            Statement::Loop { header, body, .. } => {
                match header {
                    LoopHeader::While(value) | LoopHeader::DoWhile(value) => {
                        expression(expr, value, &mut result)
                    }
                    LoopHeader::For {
                        initializer,
                        condition,
                        update,
                    } => {
                        if let Some(initializer) = initializer {
                            simple(expr, initializer, &mut result);
                        }
                        expression(expr, condition, &mut result);
                        simple(expr, update, &mut result);
                    }
                }
                result.extend(finalize(expr, body));
            }
            Statement::Switch { value, cases, .. } => {
                if let Selector::Value(value) = value {
                    value_effects(expr, ValueUse::Reference(*value), &mut result);
                }
                for case in cases {
                    result.extend(finalize(expr, &mut case.body));
                }
            }
            Statement::Synchronized { monitor, body } => {
                expression(expr, monitor, &mut result);
                result.extend(finalize(expr, body));
            }
            Statement::Block { body, .. } => result.extend(finalize(expr, body)),
            Statement::Try {
                body,
                catches,
                cleanup,
            } => {
                let mut protected = finalize(expr, body);
                let mut witnesses = Vec::new();
                for catch in catches.iter_mut() {
                    if !protected.source_caught_by(expr.cx.program, Some(&catch.catch_type)) {
                        witnesses.push(Statement::If {
                            condition: Expression::Boolean(false),
                            yes: vec![Statement::Throw(Throw::Witness(catch.ty.clone()))],
                            no: Vec::new(),
                        });
                    }
                    result.extend(finalize(expr, &mut catch.body));
                }
                // Witnesses preserve the already-owned catch without reviving a
                // call removed by source path facts or completion truncation.
                body.splice(..0, witnesses);
                for catch in catches {
                    protected.remove_caught(expr.cx.program, Some(&catch.catch_type));
                }
                result.extend(protected);
                if let Some(cleanup) = cleanup {
                    result.extend(finalize(expr, cleanup));
                }
            }
            _ => (),
        }
    }
    result
}

fn simple(expr: &ExprPlanner<'_, '_>, simple: &Simple, result: &mut Thrown) {
    match simple {
        Simple::Assign { value, .. } => expression(expr, value, result),
        Simple::Scheduled(statement) => result.extend(exceptions::statement_effects(
            expr.cx,
            expr.member,
            expr.ir,
            statement,
        )),
        _ => (),
    }
}

fn expression(expr: &ExprPlanner<'_, '_>, expression: &Expression, result: &mut Thrown) {
    super::access::for_each_value(
        expr.ir,
        expression,
        |usage| value_effects(expr, usage, result),
        |_| {},
    );
}

fn value_effects(expr: &ExprPlanner<'_, '_>, usage: ValueUse, result: &mut Thrown) {
    exceptions::expression_effects(expr.cx, expr.member, expr.ir, usage, result);
}
