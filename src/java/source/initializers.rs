//! Split bound static initialization into declarations and intervening blocks.
//! Publications stay in execution order; a local spanning a publication keeps
//! that assignment in its block until the local's last use closes the scope.
use super::{
    Body, Statement,
    bound::{Initializer, Simple},
    completion, evaluation,
};
use crate::java::{
    expr::ExprPlanner,
    syntax::{Expr, Expression},
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn extract(expr: &ExprPlanner<'_, '_>, body: &mut Body) -> Vec<Initializer<Body>> {
    let cx = expr.cx;
    let Some(member) = expr.member else {
        return Vec::new();
    };
    if cx.program.class(member.class).access_flags & 0x0200 != 0
        || !cx.program.throws_plan().boundary(member).is_empty()
    {
        return Vec::new();
    }
    let mut locals = BTreeMap::new();
    for (index, statement) in body.iter_mut().enumerate() {
        let effects = evaluation::statement_effects(statement);
        let declaration = match statement {
            Statement::Simple(Simple::Declare { binding, .. }) => Some(binding.local),
            _ => None,
        };
        for local in effects
            .occurrences()
            .map(|(local, _)| local)
            .chain(declaration)
        {
            locals.entry(local).or_insert((index, index)).1 = index;
        }
    }
    let mut local_ends = vec![0; body.len()];
    for (first, last) in locals.into_values() {
        local_ends[first] = local_ends[first].max(last + 1);
    }
    let mut fields = Vec::new();
    let mut seen = BTreeSet::new();
    let mut live_until = 0;
    let exit = Some("$initializer".to_owned());
    for (index, statement) in body.iter_mut().enumerate() {
        // A return from <clinit> exits the single enclosing label emitted by
        // context. Its continuation must remain inside that same static block.
        if completion::transfers_to(std::slice::from_ref(statement), &exit, false, false) {
            break;
        }
        live_until = live_until.max(local_ends[index]);
        if live_until > index {
            continue;
        }
        let Statement::Simple(Simple::Expression(expression)) = statement else {
            continue;
        };
        let Some((field, true)) = expression
            .facts
            .details
            .as_ref()
            .and_then(|details| details.publication)
        else {
            continue;
        };
        let declaration = cx.program.field(field);
        if field.class != member.class
            || declaration.access_flags & 0x0008 == 0
            || declaration.constant_value.is_some()
            || seen.contains(&field)
        {
            continue;
        }
        let Expression::Assignment {
            operands,
            compound: None,
        } = &mut expression.kind
        else {
            continue;
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
            continue;
        }
        seen.insert(field);
        fields.push((index, field.index));
    }
    let Some(&(last, _)) = fields.last() else {
        return Vec::new();
    };
    let mut later: BTreeSet<_> = seen
        .iter()
        .map(|field| cx.types.names.field_name(*field).to_owned())
        .collect();
    let mut fields = fields.into_iter().peekable();
    let mut block = Vec::new();
    let mut result = Vec::new();
    for (index, statement) in body.drain(..=last).enumerate() {
        if fields
            .peek()
            .is_some_and(|&(position, _)| position == index)
        {
            let (_, field) = fields.next().unwrap();
            if !block.is_empty() {
                // Earlier blocks can now precede a field's declaration. Only
                // qualify those forward references; all other bindings survive.
                evaluation::visit_expressions(&mut block, |value| {
                    qualify(expr, value, Some(&later))
                });
                result.push(Initializer::Block(std::mem::take(&mut block)));
            }
            let Statement::Simple(Simple::Expression(expression)) = statement else {
                unreachable!()
            };
            let Expression::Assignment { operands, .. } = expression.kind else {
                unreachable!()
            };
            let [_, mut value] = *operands;
            evaluation::visit_expression(&mut value, |value| qualify(expr, value, None));
            later.remove(cx.types.names.field_name(crate::program::MemberId {
                class: member.class,
                index: field,
            }));
            result.push(Initializer::Field(field, value));
        } else {
            block.push(statement);
        }
    }
    result
}

fn qualify(expr: &ExprPlanner<'_, '_>, value: &mut Expr, fields: Option<&BTreeSet<String>>) {
    // Binding uses Field(Atom) only for own unqualified static fields; locals
    // retain Local IDs and other receivers are already explicit Members.
    if let Expression::Field(reference) = &mut value.kind
        && let Expression::Atom(name) = &reference.kind
        && fields.is_none_or(|fields| fields.contains(name))
    {
        **reference = expr
            .cx
            .types
            .qualifier(&expr.cx.program.class(expr.cx.types.owner).name)
            .member(name);
    }
}
