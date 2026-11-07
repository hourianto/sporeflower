//! Case ownership is fixed before spelling: only independently entered,
//! terminal arms may share a source body. Comparison uses local/value identity,
//! never rendered text or a new evaluation of a scheduled producer.
use super::{Case, Expression, Simple, Statement};
use crate::ir::{Literal, MethodIr, ValueKind};

pub(super) fn group(ir: &MethodIr, cases: &mut Vec<Case>) {
    // A final fallthrough into an empty switch exit has the same continuation
    // as an explicit break. Settle that continuation before grouping arms.
    for index in (0..cases.len().saturating_sub(1)).rev() {
        if super::completion::can_complete(&cases[index].body)
            && matches!(cases[index + 1].body.as_slice(), [Statement::Break(None)])
        {
            cases[index].body.push(Statement::Break(None));
        }
    }
    let mut index = 0;
    while index < cases.len() {
        let independent = index == 0 || terminal(&cases[index - 1].body);
        if independent && terminal(&cases[index].body) && !cases[index].scoped {
            let earlier = (0..index).find(|&earlier| {
                !cases[earlier].scoped
                    && terminal(&cases[earlier].body)
                    && same_body(ir, &cases[earlier].body, &cases[index].body)
            });
            if let Some(earlier) = earlier {
                let case = cases.remove(index);
                cases[earlier].keys.extend(case.keys);
                cases[earlier].default |= case.default;
                continue;
            }
        }
        index += 1;
    }
}

fn terminal(body: &[Statement]) -> bool {
    matches!(
        body.last(),
        Some(Statement::Break(_) | Statement::Continue(_) | Statement::Return(_))
    )
}

fn same_body(ir: &MethodIr, left: &[Statement], right: &[Statement]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| match (left, right) {
                (Statement::Break(a), Statement::Break(b))
                | (Statement::Continue(a), Statement::Continue(b)) => a == b,
                (Statement::Return(None), Statement::Return(None)) => true,
                (Statement::Return(Some(a)), Statement::Return(Some(b))) => same_value(ir, a, b),
                (
                    Statement::Simple(Simple::Assign {
                        local: a,
                        value: av,
                        declaration: false,
                    }),
                    Statement::Simple(Simple::Assign {
                        local: b,
                        value: bv,
                        declaration: false,
                    }),
                ) => a == b && same_value(ir, av, bv),
                _ => false,
            })
}

fn same_value(ir: &MethodIr, left: &Expression, right: &Expression) -> bool {
    match (left, right) {
        (Expression::Boolean(a), Expression::Boolean(b)) => a == b,
        (Expression::Value(a), Expression::Value(b)) => match (a.local(ir), b.local(ir)) {
            (Some(a), Some(b)) => a == b,
            (None, None) => {
                let a = &ir.values[a.value()];
                let b = &ir.values[b.value()];
                a.ty == b.ty
                    && match (&a.kind, &b.kind) {
                        (ValueKind::Literal(a), ValueKind::Literal(b)) => match (a, b) {
                            (Literal::Int(a), Literal::Int(b)) => a == b,
                            (Literal::Long(a), Literal::Long(b)) => a == b,
                            (Literal::Float(a), Literal::Float(b)) => a.to_bits() == b.to_bits(),
                            (Literal::Double(a), Literal::Double(b)) => a.to_bits() == b.to_bits(),
                            (Literal::String(a), Literal::String(b)) => a == b,
                            (Literal::Null, Literal::Null) => true,
                            _ => false,
                        },
                        _ => false,
                    }
            }
            _ => false,
        },
        _ => false,
    }
}
