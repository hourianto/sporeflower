//! Lower directly into one source tree with resolved Java expressions. Scheduled
//! path and identity proofs settle while lowering; final-field analysis then
//! closes preparation. Source rewrites and layout consume that same tree.
mod bind;
pub(super) mod bound;
mod choices;
mod completion;
mod effects;
pub(super) mod evaluation;
mod facts;
mod finals;
mod lifetimes;
pub(super) mod loops;
mod lower;
mod normalize;
mod simplify;
mod switches;

use super::{
    expr::ExprPlanner,
    syntax::{Expr, Prefix},
};
use crate::{
    Result,
    classfile::JvmType,
    ir::{Literal, LocalId, ValueKind, ValueUse},
};
pub(super) use bound::Plan;
pub(super) use facts::same_local_predicate;
pub(super) use lower::Options;

pub(super) fn plan(
    expr: &ExprPlanner<'_, '_>,
    tree: &crate::structure::StructuredMethod,
    body: &[crate::structure::Stmt],
    options: Options<'_>,
) -> Result<Plan> {
    let selected = options.selected;
    let initialize = options.initialize;
    let delegates = options.delegates;
    let initializes_fields = options.initializes_fields;
    let binder = bind::Binder::new(expr, selected);
    let (mut body, caught) = lower::lower(expr, &binder, tree, body, options)?;
    finals::check(expr, &body, delegates, initializes_fields);
    let declarations = binder.finish(&mut body, &caught, initialize);
    let result = bound::ValueType {
        ty: expr.ir.return_type.clone(),
        name: expr.cx.types.ty(&expr.ir.return_type),
    };
    simplify::simplify(&mut body, &result);
    super::source_locals::place(&mut body, declarations);
    let thrown = effects::finalize(expr.cx.program, &mut body);
    Ok(bound::layout(body, thrown))
}

pub(super) type Body = Vec<Statement>;

pub(super) enum Statement {
    Simple(bound::Simple),
    Return(Option<Expr>),
    Throw(bound::Throw),
    If {
        condition: Expr,
        yes: Body,
        no: Body,
    },
    Loop {
        label: Option<String>,
        header: LoopHeader,
        body: Body,
    },
    Switch {
        label: Option<String>,
        value: bound::Selector,
        cases: Vec<Case>,
    },
    Block {
        label: Option<String>,
        body: Body,
    },
    Break(Option<String>),
    Continue(Option<String>),
    Try {
        body: Body,
        catches: Vec<Catch>,
        cleanup: Option<Body>,
    },
    Synchronized {
        monitor: Expr,
        body: Body,
    },
}

pub(super) enum LoopHeader {
    While(Expr),
    DoWhile(Expr),
    For {
        initializer: Option<Box<bound::Simple>>,
        condition: Expr,
        update: Box<bound::Simple>,
    },
}

pub(super) struct Case {
    pub keys: Vec<i32>,
    pub default: bool,
    pub scoped: bool,
    pub body: Body,
}

pub(super) struct Catch {
    pub local: Option<LocalId>,
    pub catch_type: String,
    pub ty: String,
    pub name: String,
    pub body: Body,
}

/// A truth test observes nonzero bits. A value preserves its raw boolean slot;
/// a selected arm also needs the exact consumer type before Java combines arms.
#[derive(Clone, Copy)]
pub(super) enum Context<'a> {
    Test,
    Assignment(&'a JvmType),
    Operand(&'a JvmType),
}

impl<'a> Context<'a> {
    pub(super) fn ty(self) -> &'a JvmType {
        match self {
            Self::Test => &JvmType::Boolean,
            Self::Assignment(ty) | Self::Operand(ty) => ty,
        }
    }

    fn as_value(self) -> bool {
        !matches!(self, Self::Test)
    }

    fn arm(self) -> Self {
        match self {
            Self::Assignment(ty) => Self::Operand(ty),
            context => context,
        }
    }
}

/// Bind directly from the scheduled producer into the one source expression tree.
/// Test projection may consume any nonzero int; value projection preserves raw
/// boolean slots and gives conditional arms their exact consumer type.
pub(in crate::java) fn value(
    expr: &ExprPlanner<'_, '_>,
    usage: ValueUse,
    context: Context<'_>,
    negated: bool,
) -> Result<Expr> {
    bind_value(expr, usage, context, negated, &facts::Facts::default(), 0)
}

fn bind_value(
    expr: &ExprPlanner<'_, '_>,
    usage: ValueUse,
    context: Context<'_>,
    negated: bool,
    facts: &facts::Facts,
    depth: usize,
) -> Result<Expr> {
    anyhow::ensure!(
        depth < 512,
        "boolean expression nesting exceeds scheduled depth"
    );
    if *context.ty() == JvmType::Boolean && usage.local(expr.ir).is_none() {
        match &expr.ir.values[usage.value()].kind {
            ValueKind::Literal(Literal::Int(value))
                if !context.as_value() || matches!(value, 0 | 1) =>
            {
                return Ok(Expr::boolean((*value != 0) ^ negated));
            }
            ValueKind::Alias(inner) => {
                return bind_value(
                    expr,
                    ValueUse::Reference(*inner),
                    context,
                    negated,
                    facts,
                    depth + 1,
                );
            }
            ValueKind::Select {
                condition,
                then_value,
                else_value,
            } => {
                let arm = |value, truth, context, negated| {
                    bind_value(
                        expr,
                        ValueUse::Reference(value),
                        context,
                        negated,
                        &facts.path(expr, condition, truth),
                        depth + 1,
                    )
                };
                if let Some(truth) = facts.condition(expr, condition) {
                    return arm(
                        if truth { *then_value } else { *else_value },
                        truth,
                        context,
                        negated,
                    );
                }
                let yes = arm(*then_value, true, context.arm(), negated)?;
                let no = arm(*else_value, false, context.arm(), negated)?;
                if let (Some(yes), Some(no)) = (yes.boolean_constant(), no.boolean_constant())
                    && yes != no
                {
                    return facts.bind_condition(expr, condition, !yes, context.as_value());
                }
                let logical = match (yes.boolean_constant(), no.boolean_constant()) {
                    (Some(value), _) if !context.as_value() || no.canonical_boolean() => {
                        Some((!value, value, no.clone()))
                    }
                    (_, Some(value)) if !context.as_value() || yes.canonical_boolean() => {
                        Some((value, value, yes.clone()))
                    }
                    _ => None,
                };
                if let Some((inverse, disjunction, mut other)) = logical {
                    // Choosing a logical form gives its arm a truth consumer,
                    // including delegation arguments rendered without a body.
                    normalize::normalize_test(&mut other);
                    return Ok(Expr::logical(
                        !disjunction,
                        vec![
                            facts.bind_condition(expr, condition, inverse, false)?,
                            other,
                        ],
                    ));
                }
                return Ok(Expr::conditional(
                    facts.bind_condition(expr, condition, false, false)?,
                    yes,
                    no,
                ));
            }
            _ => (),
        }
    }
    let value = expr.source_value(usage, context)?;
    Ok(if negated {
        Expr::prefix(Prefix::Not, value)
    } else {
        value
    })
}
