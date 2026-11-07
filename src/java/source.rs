//! Source planning has one entry point. Structural lowering retains scheduled
//! identities until declarations, idioms and path facts settle. Checked effects
//! and all Java expression bindings are then fixed into an IR-free syntax plan.
pub(super) mod access;
mod bind;
mod completion;
mod effects;
mod facts;
mod finals;
pub(super) mod loops;
mod lower;
mod switches;

use super::{
    expr::{ExprPlanner, comparison},
    syntax::{Expr, Prefix},
};
use crate::{
    Result,
    classfile::JvmType,
    ir::{CompareOp, Condition, Literal, LocalId, MethodIr, ValueId, ValueKind, ValueUse},
    structure::Predicate,
};
pub(super) use bind::Plan;
pub(super) use facts::same_local_predicate;
pub(super) use lower::Options;

pub(super) fn plan(
    expr: &ExprPlanner<'_, '_>,
    tree: &crate::structure::StructuredMethod,
    body: &[crate::structure::Stmt],
    options: Options<'_>,
) -> Result<Plan> {
    let delegates = options.delegates;
    let initializes_fields = options.initializes_fields;
    let mut body = lower::lower(expr, tree, body, options)?;
    let thrown = effects::finalize(expr, &mut body);
    finals::check(expr, &body, delegates, initializes_fields);
    bind::bind(expr, &body, thrown)
}

pub(super) type Body = Vec<Statement>;

pub(super) enum Statement {
    Simple(Simple),
    Return(Option<Expression>),
    Throw(Throw),
    If {
        condition: Expression,
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
        value: Selector,
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
        monitor: Expression,
        body: Body,
    },
}

pub(super) enum Selector {
    Value(ValueId),
    Dispatch(String),
}

pub(super) enum Simple {
    /// A declaration without an initializer, or one initialized for dispatch.
    Declare { local: LocalId, initialize: bool },
    Assign {
        local: LocalId,
        value: Expression,
        declaration: bool,
    },
    /// Scheduled effects retain their value identities until expression binding.
    Scheduled(crate::ir::Statement),
    CatchBind {
        local: LocalId,
        name: String,
        ty: Option<String>,
    },
    Dispatch {
        state: String,
        target: crate::ir::BlockId,
        declare: bool,
    },
}

pub(super) enum Throw {
    Value { value: ValueId, pc: u32 },
    Witness(String),
    InvalidDispatch,
}

pub(super) enum LoopHeader {
    While(Expression),
    DoWhile(Expression),
    For {
        initializer: Option<Simple>,
        condition: Expression,
        update: Box<Simple>,
    },
}

pub(super) struct Case {
    pub keys: Vec<i32>,
    pub default: bool,
    pub scoped: bool,
    pub body: Body,
}

pub(super) struct Catch {
    pub catch_type: String,
    pub ty: String,
    pub name: String,
    pub body: Body,
}

#[derive(Clone)]
pub(super) enum Expression {
    Value(ValueUse),
    Boolean(bool),
    Update {
        local: LocalId,
        increment: bool,
    },
    Compare {
        op: CompareOp,
        left: Box<Expression>,
        right: Box<Expression>,
    },
    Condition(Condition),
    Predicate {
        predicate: Predicate,
        negated: bool,
    },
    Select {
        condition: Box<Expression>,
        yes: Box<Expression>,
        no: Box<Expression>,
    },
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

impl Expression {
    pub fn value(ir: &MethodIr, usage: ValueUse, ty: &JvmType) -> Self {
        if *ty == JvmType::Boolean && usage.local(ir).is_none() {
            match ir.values[usage.value()].kind {
                ValueKind::Literal(Literal::Int(0)) => return Self::Boolean(false),
                ValueKind::Literal(Literal::Int(1)) => return Self::Boolean(true),
                ValueKind::Alias(inner) => return Self::value(ir, ValueUse::Reference(inner), ty),
                _ => {}
            }
        }
        Self::Value(usage)
    }

    fn boolean(ir: &MethodIr, usage: ValueUse, context: Context<'_>, depth: usize) -> Result<Self> {
        anyhow::ensure!(
            depth < 512,
            "boolean expression nesting exceeds scheduled depth"
        );
        if usage.local(ir).is_none() {
            match &ir.values[usage.value()].kind {
                ValueKind::Literal(Literal::Int(value)) if !context.as_value() => {
                    return Ok(Self::Boolean(*value != 0));
                }
                ValueKind::Alias(value) => {
                    return Self::boolean(ir, ValueUse::Reference(*value), context, depth + 1);
                }
                ValueKind::Select {
                    condition,
                    then_value,
                    else_value,
                } => {
                    return Ok(Self::select(
                        Self::Condition(condition.clone()),
                        Self::boolean(ir, ValueUse::Reference(*then_value), context, depth + 1)?,
                        Self::boolean(ir, ValueUse::Reference(*else_value), context, depth + 1)?,
                    ));
                }
                _ => {}
            }
        }
        Ok(Self::value(ir, usage, &JvmType::Boolean))
    }

    pub fn predicate(predicate: &Predicate, negated: bool) -> Self {
        Self::Predicate {
            predicate: predicate.clone(),
            negated,
        }
    }

    fn negate(self) -> Self {
        Self::select(self, Self::Boolean(false), Self::Boolean(true))
    }

    fn canonical(&self, expr: &ExprPlanner<'_, '_>) -> bool {
        match self {
            Self::Value(usage) => {
                let ty = usage
                    .local(expr.ir)
                    .map_or(&expr.ir.values[usage.value()].ty, |local| {
                        &expr.ir.locals[local].ty
                    });
                // An int used in boolean source context is explicitly compared
                // with zero. A boolean slot itself can still contain other ints.
                *ty != JvmType::Boolean || expr.canonical_boolean(*usage)
            }
            Self::Select { yes, no, .. } => yes.canonical(expr) && no.canonical(expr),
            Self::Boolean(_)
            | Self::Compare { .. }
            | Self::Condition(_)
            | Self::Predicate { .. } => true,
            Self::Update { .. } => false,
        }
    }

    fn select(condition: Self, yes: Self, no: Self) -> Self {
        Self::Select {
            condition: Box::new(condition),
            yes: Box::new(yes),
            no: Box::new(no),
        }
    }

    fn selection_fits(expr: &ExprPlanner<'_, '_>, condition: &Self, yes: &Self, no: &Self) -> bool {
        fn fits(
            expr: &ExprPlanner<'_, '_>,
            value: &Expression,
            budget: &mut usize,
            choices: usize,
        ) -> bool {
            let Some(left) = budget.checked_sub(1) else {
                return false;
            };
            *budget = left;
            match value {
                Expression::Select { condition, yes, no } => {
                    let logical = matches!(yes.as_ref(), Expression::Boolean(_))
                        && no.canonical(expr)
                        || matches!(no.as_ref(), Expression::Boolean(_)) && yes.canonical(expr);
                    let choices = choices + usize::from(!logical);
                    choices <= 3
                        && fits(expr, condition, budget, choices)
                        && fits(expr, yes, budget, choices)
                        && fits(expr, no, budget, choices)
                }
                _ => true,
            }
        }
        // Sequential early returns can contain thousands of guards. Keep the
        // source decision tree readable and its new recursion independently
        // bounded, even when the structured input is a flat statement list.
        let mut budget = 48;
        let truth_fold = matches!(yes, Self::Boolean(_)) && no.canonical(expr)
            || matches!(no, Self::Boolean(_)) && yes.canonical(expr);
        let choices = usize::from(!truth_fold);
        fits(expr, condition, &mut budget, 0)
            && fits(expr, yes, &mut budget, choices)
            && fits(expr, no, &mut budget, choices)
    }

    pub fn plan(&self, expr: &ExprPlanner<'_, '_>, ty: &JvmType) -> Result<Expr> {
        self.bind(expr, Context::Assignment(ty), false)
    }

    pub fn condition(&self, expr: &ExprPlanner<'_, '_>) -> Result<Expr> {
        self.bind(expr, Context::Test, false)
    }

    pub(in crate::java) fn bind(
        &self,
        expr: &ExprPlanner<'_, '_>,
        context: Context<'_>,
        negated: bool,
    ) -> Result<Expr> {
        self.bind_with_facts(expr, context, negated, &facts::Facts::default())
    }

    fn bind_with_facts(
        &self,
        expr: &ExprPlanner<'_, '_>,
        context: Context<'_>,
        negated: bool,
        facts: &facts::Facts,
    ) -> Result<Expr> {
        let expression = match self {
            Self::Value(usage) => {
                if *context.ty() == JvmType::Boolean
                    && usage.local(expr.ir).is_none()
                    && matches!(expr.ir.values[usage.value()].kind, ValueKind::Select { .. })
                {
                    return Self::boolean(expr.ir, *usage, context, 0)?
                        .bind_with_facts(expr, context, negated, facts);
                }
                let value = expr.source_value(*usage, context)?;
                return Ok(if negated {
                    Expr::prefix(Prefix::Not, value)
                } else {
                    value
                });
            }
            Self::Boolean(value) => Expr::atom((value ^ negated).to_string()),
            Self::Update { local, increment } => {
                Expr::update(Expr::atom(&expr.ir.locals[*local].name), *increment, false)
            }
            Self::Condition(condition) => {
                return facts.bind_condition(expr, condition, negated, context.as_value());
            }
            Self::Compare { op, left, right } => Expr::binary(
                comparison(if negated { op.inverted() } else { *op }),
                left.bind(expr, Context::Operand(&JvmType::Int), false)?,
                right.bind(expr, Context::Operand(&JvmType::Int), false)?,
            ),
            Self::Predicate {
                predicate,
                negated: inverted,
            } => {
                if let Predicate::Atom {
                    head,
                    negated: atom_negated,
                } = predicate
                    && let crate::ir::Terminator::Branch { condition, .. } =
                        &expr.ir.blocks[*head].terminator
                {
                    return facts.bind_condition(
                        expr,
                        condition,
                        negated ^ inverted ^ atom_negated,
                        context.as_value(),
                    );
                }
                return expr.predicate(predicate, negated ^ inverted, context.as_value());
            }
            Self::Select { condition, yes, no } => {
                if let Self::Predicate {
                    predicate: Predicate::And(parts),
                    negated: false,
                } = condition.as_ref()
                    && let [guard, arm] = parts.as_slice()
                    && let Self::Boolean(value) = yes.as_ref()
                {
                    let selected = Self::predicate(guard, false);
                    let mut guarded = facts.path(expr, &selected, true);
                    guarded.predicate(expr, arm);
                    if guarded.expression(expr, no) == Some(!value) {
                        let alternative = no.bind_with_facts(
                            expr,
                            context.arm(),
                            negated,
                            &facts.path(expr, &selected, false),
                        )?;
                        // Only an alternative eligible for logical projection
                        // was a truth value. Other selected results keep raw bits.
                        let alternative = if no.canonical(expr) {
                            alternative.truth_value()
                        } else {
                            alternative
                        };
                        return Ok(Expr::conditional(
                            expr.predicate(guard, false, false)?,
                            expr.predicate(arm, !value ^ negated, context.as_value())?
                                .truth_value(),
                            alternative,
                        ));
                    }
                }
                if *context.ty() == JvmType::Boolean {
                    if let (Self::Boolean(yes), Self::Boolean(no)) = (yes.as_ref(), no.as_ref())
                        && yes != no
                    {
                        return condition.bind_with_facts(
                            expr,
                            context.arm(),
                            !yes ^ negated,
                            facts,
                        );
                    }
                    let logical = match (yes.as_ref(), no.as_ref()) {
                        (Self::Boolean(value), arm)
                            if !context.as_value() || arm.canonical(expr) =>
                        {
                            Some((!value, *value, arm))
                        }
                        (arm, Self::Boolean(value))
                            if !context.as_value() || arm.canonical(expr) =>
                        {
                            Some((*value, *value, arm))
                        }
                        _ => None,
                    };
                    if let Some((inverse, disjunction, arm)) = logical {
                        return Ok(Expr::logical(
                            !(disjunction ^ negated),
                            vec![
                                condition.bind_with_facts(
                                    expr,
                                    Context::Test,
                                    inverse ^ negated,
                                    facts,
                                )?,
                                arm.bind_with_facts(
                                    expr,
                                    Context::Test,
                                    negated,
                                    &facts.path(expr, condition, inverse == disjunction),
                                )?,
                            ],
                        ));
                    }
                }
                Expr::conditional(
                    condition.bind_with_facts(expr, Context::Test, false, facts)?,
                    yes.bind_with_facts(
                        expr,
                        context.arm(),
                        negated,
                        &facts.path(expr, condition, true),
                    )?,
                    no.bind_with_facts(
                        expr,
                        context.arm(),
                        negated,
                        &facts.path(expr, condition, false),
                    )?,
                )
            }
        };
        Ok(expression)
    }
}

/// Finish one lexical source sequence as its children arrive. These are local
/// source idioms, not another control-flow recovery or a fixed-point repair pass.
fn append(body: &mut Body, statement: Statement, expr: &ExprPlanner<'_, '_>) {
    let statement = match statement {
        Statement::If {
            condition,
            mut yes,
            mut no,
        } if yes.len() == 1 && no.len() == 1 => match (&yes[0], &no[0]) {
            (Statement::Return(Some(yes_value)), Statement::Return(Some(no_value)))
                if Expression::selection_fits(expr, &condition, yes_value, no_value) =>
            {
                let Statement::Return(Some(yes)) = yes.pop().unwrap() else {
                    unreachable!()
                };
                let Statement::Return(Some(no)) = no.pop().unwrap() else {
                    unreachable!()
                };
                Statement::Return(Some(Expression::select(condition, yes, no)))
            }
            (
                Statement::Simple(Simple::Assign {
                    local: a,
                    declaration: false,
                    value: yes_value,
                }),
                Statement::Simple(Simple::Assign {
                    local: b,
                    declaration: false,
                    value: no_value,
                }),
            ) if a == b && Expression::selection_fits(expr, &condition, yes_value, no_value) => {
                let Statement::Simple(Simple::Assign {
                    local, value: yes, ..
                }) = yes.pop().unwrap()
                else {
                    unreachable!()
                };
                let Statement::Simple(Simple::Assign { value: no, .. }) = no.pop().unwrap() else {
                    unreachable!()
                };
                Statement::Simple(Simple::Assign {
                    local,
                    value: Expression::select(condition, yes, no),
                    declaration: false,
                })
            }
            _ => Statement::If { condition, yes, no },
        },
        statement => statement,
    };
    if let Statement::Return(Some(no)) = statement {
        if body.last().is_some_and(|statement| matches!(statement,
            Statement::If { condition, yes, no: else_body }
                if else_body.is_empty() && matches!(yes.as_slice(), [Statement::Return(Some(yes))] if Expression::selection_fits(expr, condition, yes, &no))))
        {
            let Statement::If {
                condition, mut yes, ..
            } = body.pop().unwrap()
            else {
                unreachable!()
            };
            let Statement::Return(Some(yes)) = yes.pop().unwrap() else {
                unreachable!()
            };
            append(
                body,
                Statement::Return(Some(Expression::select(condition, yes, no))),
                expr,
            );
        } else {
            body.push(Statement::Return(Some(no)));
        }
    } else {
        body.push(statement);
    }
}
