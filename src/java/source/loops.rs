//! Preparation owns exact scheduled initializer/update sites and expansion of
//! larger finite controls. Bound presentation settles exhaustion guards, private
//! zero/one iteration controls, tail filters and post-test spelling together.
use super::{Body, LoopHeader, Statement as Source, bind, bound::Simple};
use crate::java::syntax::{Expr, Expression};
use crate::java::{expr::ExprPlanner, source_updates};
use crate::{
    Result,
    classfile::JvmType,
    ir::{BlockId, LocalId, Statement, Terminator, ValueUse},
    structure::{Predicate, RegionId, Stmt},
};

mod repetition;
pub(super) use repetition::{accesses as control_accesses, expand as expand_finite};

pub(super) type Site = (BlockId, usize);

/// Presentation depends on the completed Java expressions. In particular,
/// substitution can expose the return that supplies an exhaustion continuation.
/// Scheduled update/repetition proofs remain in preparation below.
pub(super) fn present_statement(
    statement: super::bound::Statement,
    accesses: &std::collections::BTreeMap<LocalId, usize>,
) -> super::bound::Body {
    use super::bound::{LoopHeader, Simple, Statement};
    use crate::java::syntax::Expression as BoundExpression;
    let Statement::Loop {
        label,
        mut header,
        mut body,
    } = statement
    else {
        return vec![statement];
    };
    remove_tail_continues(&mut body, &label);
    let mut before = Vec::new();
    let mut outward = Vec::new();
    let unconditional = match &header {
        LoopHeader::While(condition) | LoopHeader::For { condition, .. } => {
            condition.boolean_constant() == Some(true)
        }
        LoopHeader::DoWhile(_) => false,
    };
    if unconditional
        && let Some(Statement::If { yes, no, .. }) = body.first()
        && (no.is_empty() && matches!(yes.as_slice(), [Statement::Return(_)])
            || yes.is_empty() && matches!(no.as_slice(), [Statement::Return(_)]))
        && !super::completion::breaks_loop(&body[1..], &label)
    {
        let Statement::If {
            condition,
            mut yes,
            mut no,
        } = body.remove(0)
        else {
            unreachable!()
        };
        let (condition, exit) = if no.is_empty() {
            (condition.negated_test(), yes.pop().unwrap())
        } else {
            (condition, no.pop().unwrap())
        };
        match &mut header {
            LoopHeader::While(value)
            | LoopHeader::For {
                condition: value, ..
            } => *value = condition,
            _ => unreachable!(),
        }
        outward.push(exit);
    }
    // A single action after a simple skip test has a direct positive guard.
    // Compound conditions and larger suffixes retain their existing nesting.
    let end = body.len();
    if end >= 2
        && matches!(
            &body[end - 1],
            Statement::Simple(
                Simple::Expression(_)
                    | Simple::Assign {
                        declaration: false,
                        ..
                    }
            )
        )
        && matches!(&body[end - 2], Statement::If {condition, yes, no}
            if !matches!(condition.kind, BoundExpression::Logical {..} | BoundExpression::Conditional(_))
                && matches!(yes.as_slice(), [Statement::Continue(None)]) && no.is_empty())
    {
        let action = body.pop().unwrap();
        let Statement::If { condition, .. } = body.pop().unwrap() else {
            unreachable!()
        };
        body.push(Statement::If {
            condition: condition.negated_test(),
            yes: vec![action],
            no: Vec::new(),
        });
    }
    if let Some(once) = trivial_repetition(&header, accesses)
        && (!once
            || !super::completion::targets_loop(&body, &label, false)
                && !super::completion::targets_loop(&body, &label, true))
    {
        if once {
            before.extend(body);
        }
        if super::completion::can_complete(&before) {
            before.extend(outward);
        }
        return before;
    }
    if let Some((condition, end)) = post_test(&header, &body, &label)
        && !super::completion::targets_loop(&body[..end], &label, true)
    {
        header = LoopHeader::DoWhile(condition);
        body.truncate(end);
    }
    before.push(Statement::Loop {
        label,
        header,
        body,
    });
    before.extend(outward);
    before
}

/// Normal completion already takes the owning loop's update/condition edge.
/// Only lexical tails share that edge. A finally may override a pending exit,
/// and nested loops or switch arms have their own fallthrough destinations.
fn remove_tail_continues(body: &mut Body, label: &Option<String>) {
    if matches!(body.last(), Some(Source::Continue(target)) if target.is_none() || target == label)
    {
        body.pop();
    }
    match body.last_mut() {
        Some(Source::If { yes, no, .. }) => {
            remove_tail_continues(yes, label);
            remove_tail_continues(no, label);
        }
        Some(Source::Block { body, .. } | Source::Synchronized { body, .. }) => {
            remove_tail_continues(body, label);
        }
        Some(Source::Try { body, catches, .. }) => {
            remove_tail_continues(body, label);
            for catch in catches {
                remove_tail_continues(&mut catch.body, label);
            }
        }
        _ => (),
    }
}

/// A private int counter is used only by its initializer, guard and update.
/// Four semantic accesses are one initializer write, one guard read, and the
/// update's read/write pair. All body, catch and outward uses are counted too.
fn trivial_repetition(
    header: &super::bound::LoopHeader,
    accesses: &std::collections::BTreeMap<LocalId, usize>,
) -> Option<bool> {
    use super::bound::{AssignmentValue, LoopHeader, Simple};
    use crate::java::syntax::{Expression, Operator};
    let LoopHeader::For {
        initializer: Some(initializer),
        condition,
        update,
    } = header
    else {
        return None;
    };
    let Simple::Assign {
        binding,
        value: AssignmentValue::Value(value),
        ..
    } = initializer.as_ref()
    else {
        return None;
    };
    if binding.ty != JvmType::Int || accesses.get(&binding.local) != Some(&4) {
        return None;
    }
    let start = bound_integer(value)?;
    let Expression::Binary(operator, operands) = &condition.kind else {
        return None;
    };
    if !matches!(operands[0].kind, Expression::Local {id, ..} if id == binding.local) {
        return None;
    }
    let limit = bound_integer(&operands[1])?;
    let Simple::Assign {
        binding: updated,
        value,
        ..
    } = update.as_ref()
    else {
        return None;
    };
    if updated.local != binding.local {
        return None;
    }
    let step = assigned_step(updated, value)?;
    let test = |value| {
        Some(match operator {
            Operator::Equal => value == limit,
            Operator::NotEqual => value != limit,
            Operator::Less => value < limit,
            Operator::LessEqual => value <= limit,
            Operator::Greater => value > limit,
            Operator::GreaterEqual => value >= limit,
            _ => return None,
        })
    };
    let once = test(start)?;
    (!once || !test(start.wrapping_add(step))?).then_some(once)
}

fn assigned_step(
    binding: &super::bound::Binding,
    value: &super::bound::AssignmentValue,
) -> Option<i32> {
    use super::bound::AssignmentValue;
    use crate::java::syntax::Expression;
    Some(match value {
        AssignmentValue::Increment { increment } => {
            if *increment {
                1
            } else {
                -1
            }
        }
        AssignmentValue::Compound { operator, value } => {
            signed_step(*operator, bound_integer(value)?)?
        }
        AssignmentValue::Value(value) => {
            let Expression::Binary(operator, operands) = &value.kind else {
                return None;
            };
            if !matches!(operands[0].kind, Expression::Local {id, ..} if id == binding.local) {
                return None;
            }
            signed_step(*operator, bound_integer(&operands[1])?)?
        }
    })
}

fn signed_step(operator: crate::java::syntax::Operator, value: i32) -> Option<i32> {
    use crate::java::syntax::Operator;
    match operator {
        Operator::Add => Some(value),
        Operator::Subtract => Some(value.wrapping_neg()),
        _ => None,
    }
}

fn bound_integer(value: &crate::java::syntax::Expr) -> Option<i32> {
    (value.source_type() == Some(&JvmType::Int))
        .then(|| {
            value
                .integer_value()
                .and_then(|value| i32::try_from(value).ok())
        })
        .flatten()
}

fn post_test(
    header: &super::bound::LoopHeader,
    body: &[super::bound::Statement],
    label: &Option<String>,
) -> Option<(crate::java::syntax::Expr, usize)> {
    use super::bound::{LoopHeader, Statement};
    if !matches!(header, LoopHeader::While(condition) if condition.boolean_constant() == Some(true))
    {
        return None;
    }
    // The terminal decision owns the repeat/exit choice, whether recovery
    // spelled its repeat as continue or left it as ordinary fallthrough.
    match body {
        [
            ..,
            Statement::If { condition, yes, no },
            Statement::Break(target),
        ] if (target.is_none() || target == label)
            && no.is_empty()
            && matches!(yes.as_slice(), [Statement::Continue(None)]) =>
        {
            Some((condition.clone(), body.len() - 2))
        }
        [.., Statement::If { condition, yes, no }]
            if no.is_empty()
                && matches!(yes.as_slice(), [Statement::Break(target)] if target.is_none() || target == label) =>
        {
            Some((condition.clone().negated_test(), body.len() - 1))
        }
        _ => None,
    }
}

pub(super) struct Parts {
    pub header: LoopHeader,
    pub body_start: usize,
    pub initializer: Option<Site>,
    pub update: Option<Site>,
}

impl Parts {
    pub fn recognize(
        binder: &bind::Binder<'_, '_, '_>,
        region: RegionId,
        header: BlockId,
        body: &[Stmt],
        continuation: Option<BlockId>,
        prior: Option<&Stmt>,
        omitted: Option<Site>,
    ) -> Result<Self> {
        let expr = binder.expr;
        let (condition, body_start) = guard(expr, body, region)?;
        let Some(step) = source_updates::loop_step(expr, body, header, body_start, continuation)?
        else {
            return Ok(Self {
                header: LoopHeader::While(condition),
                body_start,
                initializer: None,
                update: None,
            });
        };
        let initializer = prior
            .and_then(|prior| prior.scheduled(expr.ir))
            .and_then(|scheduled| {
                let (index, Statement::Assign { local, value }) = scheduled.iter().last()? else {
                    return None;
                };
                let site = (scheduled.block, index);
                if *local != step.local || omitted == Some(site) {
                    return None;
                }
                Some((site, *local, *value))
            });
        let initializer = initializer
            .map(|(site, local, usage)| {
                binder
                    .assign(local, usage, false)
                    .map(|value| (site, value))
            })
            .transpose()?;
        let (initializer_site, initializer) = match initializer {
            Some((site, initializer)) => (Some(site), Some(Box::new(initializer))),
            None => (None, None),
        };
        Ok(Self {
            header: LoopHeader::For {
                initializer,
                condition,
                update: Box::new(
                    binder.scheduled(&expr.ir.blocks[step.block].statements[step.statement])?,
                ),
            },
            body_start,
            initializer: initializer_site,
            update: Some((step.block, step.statement)),
        })
    }
}

fn guard(expr: &ExprPlanner<'_, '_>, body: &[Stmt], region: RegionId) -> Result<(Expr, usize)> {
    if let Some(condition) = updated_guard(expr, body, region) {
        return Ok((condition?, 2));
    }
    let mut index = 0;
    if let Some(Stmt::Basic(block)) = body.first()
        && expr.ir.blocks[*block].statements.is_empty()
    {
        index = 1;
    }
    if let Some(Stmt::If {
        condition,
        then_body,
        else_body,
    }) = body.get(index)
    {
        let exits =
            |branch: &[Stmt]| matches!(branch, [Stmt::Break { target }] if *target == region);
        if exits(then_body) && else_body.is_empty() {
            return Ok((
                super::facts::Facts::default().bind_predicate(expr, condition, true)?,
                index + 1,
            ));
        }
        if then_body.is_empty() && exits(else_body) {
            return Ok((
                super::facts::Facts::default().bind_predicate(expr, condition, false)?,
                index + 1,
            ));
        }
    }
    Ok((Expr::boolean(true), 0))
}

/// The condition owns this exact leading assignment. A continue already jumps
/// to it, unlike moving an arbitrary trailing update into a loop header.
fn updated_guard(
    expr: &ExprPlanner<'_, '_>,
    body: &[Stmt],
    region: RegionId,
) -> Option<Result<Expr>> {
    let scheduled = body.first()?.scheduled(expr.ir)?;
    let mut statements = scheduled.iter();
    let (_, Statement::Assign { local, value }) = statements.next()? else {
        return None;
    };
    if statements.next().is_some() {
        return None;
    }
    let (local, value) = (*local, *value);
    let increment = match source_updates::integer_step(expr.ir, local, value)? {
        1 => true,
        -1 => false,
        _ => return None,
    };
    let Stmt::If {
        condition: Predicate::Atom { head, negated },
        then_body,
        else_body,
    } = body.get(1)?
    else {
        return None;
    };
    let exits = |branch: &[Stmt]| matches!(branch, [Stmt::Break { target }] if *target == region);
    let invert = if exits(then_body) && else_body.is_empty() {
        true
    } else if then_body.is_empty() && exits(else_body) {
        false
    } else {
        return None;
    };
    let Terminator::Branch { condition, .. } = &expr.ir.blocks[*head].terminator else {
        return None;
    };
    if expr.ir.values[condition.left].local != Some(local)
        || !matches!(
            crate::java::conversions::source_type(expr.ir, condition.right),
            JvmType::Int | JvmType::Byte | JvmType::Short | JvmType::Char
        )
    {
        return None;
    }
    let op = if invert ^ negated {
        condition.op.inverted()
    } else {
        condition.op
    };
    Some(
        super::value(
            expr,
            ValueUse::Reference(condition.right),
            super::Context::Operand(&JvmType::Int),
            false,
        )
        .map(|right| {
            Expr::binary(
                crate::java::expr::comparison(op),
                Expr::update(
                    Expr::local(local, expr.local_name(local), &expr.ir.locals[local].ty),
                    increment,
                    false,
                ),
                right,
            )
        }),
    )
}

#[cfg(test)]
#[path = "../../../tests/unit/source_loop_controls.rs"]
mod tests;
