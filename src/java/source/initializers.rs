//! Publish a leading sequence of bound static assignments at their declarations.
//! Moving only this prefix keeps exception scopes, local scopes and every later
//! effect in the original initializer. The field writer retains publication order.
use super::{Body, Statement, bound::Simple, evaluation};
use crate::java::{
    expr::ExprPlanner,
    syntax::{Expr, Expression},
};
use std::collections::BTreeSet;

pub(super) fn extract(expr: &ExprPlanner<'_, '_>, body: &mut Body) -> Vec<(usize, Expr)> {
    let cx = expr.cx;
    let Some(member) = expr.member else {
        return Vec::new();
    };
    if cx.program.class(member.class).access_flags & 0x0200 != 0
        || !cx.program.throws_plan().boundary(member).is_empty()
    {
        return Vec::new();
    }
    let mut fields = Vec::new();
    let mut seen = BTreeSet::new();
    for statement in body.iter_mut() {
        let Statement::Simple(Simple::Expression(expression)) = statement else {
            break;
        };
        let Some((field, true)) = expression
            .facts
            .details
            .as_ref()
            .and_then(|details| details.publication)
        else {
            break;
        };
        let declaration = cx.program.field(field);
        if field.class != member.class
            || declaration.access_flags & 0x0008 == 0
            || declaration.constant_value.is_some()
            || seen.contains(&field)
        {
            break;
        }
        let Expression::Assignment {
            operands,
            compound: None,
        } = &mut expression.kind
        else {
            break;
        };
        let mut local_or_write = false;
        let mut checked = false;
        let mut nonconstant = false;
        evaluation::visit_expression(&mut operands[1], |value| {
            local_or_write |= matches!(
                value.kind,
                Expression::Local { .. }
                    | Expression::Assignment { .. }
                    | Expression::Update { .. }
            );
            checked |= value.facts.details.as_ref().is_some_and(|details| {
                details.throws.source_unknown
                    || details
                        .throws
                        .source_types
                        .iter()
                        .any(|ty| crate::throws::is_checked(cx.program, ty))
            });
            // These forms cannot occur in a Java constant expression. In
            // particular, a bare field name alone is not sufficient evidence.
            nonconstant |= matches!(
                value.kind,
                Expression::Null
                    | Expression::Invocation { .. }
                    | Expression::New { .. }
                    | Expression::NewArray { .. }
                    | Expression::ArrayInitializer { .. }
                    | Expression::Index(_)
                    | Expression::InstanceOf { .. }
            );
        });
        let constant_type = matches!(
            declaration.descriptor.as_str(),
            "Z" | "B" | "C" | "S" | "I" | "J" | "F" | "D" | "Ljava/lang/String;"
        );
        // A new ConstantValue would be installed before superclass initialization
        // and could be inlined by callers, unlike the original PUTSTATIC.
        if local_or_write || checked || cx.retains_final(field) && constant_type && !nonconstant {
            break;
        }
        seen.insert(field);
        fields.push(field.index);
    }
    body.drain(..fields.len())
        .zip(fields)
        .map(|(statement, field)| {
            let Statement::Simple(Simple::Expression(expression)) = statement else {
                unreachable!()
            };
            let Expression::Assignment { operands, .. } = expression.kind else {
                unreachable!()
            };
            let [_, mut value] = *operands;
            evaluation::visit_expression(&mut value, |value| {
                // Binding uses Field(Atom) only for own unqualified static fields;
                // locals retain Local IDs and other receivers are explicit Members.
                // Declarations can precede those fields, unlike the old static block.
                if let Expression::Field(reference) = &mut value.kind
                    && let Expression::Atom(name) = &reference.kind
                {
                    **reference = cx
                        .types
                        .qualifier(&cx.program.class(member.class).name)
                        .member(name);
                }
            });
            (field, value)
        })
        .collect()
}
