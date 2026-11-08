//! Preparation owns exact scheduled initializer/update sites and expansion of
//! larger finite controls. Bound presentation settles exhaustion guards, private
//! zero/one iteration controls, tail filters and post-test spelling together.
use super::{Body, Expression, LoopHeader, Simple, Statement as Source};
use crate::java::{expr::ExprPlanner, source_updates};
use crate::{
    Result,
    classfile::JvmType,
    ir::{BlockId, Literal, LocalId, MethodIr, Statement, Terminator, ValueKind, ValueUse},
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
        initializer:
            Some(Simple::Assign {
                binding,
                value: AssignmentValue::Value(value),
                ..
            }),
        condition,
        update,
    } = header
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
    let step = match value {
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
    };
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

// The scheduled expansion proof below retains its original producer operands.
fn integer(ir: &MethodIr, usage: ValueUse) -> Option<i32> {
    let value = &ir.values[usage.value()];
    match value.kind {
        ValueKind::Literal(Literal::Int(number)) if usage.local(ir).is_none() => Some(number),
        _ => None,
    }
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
        expr: &ExprPlanner<'_, '_>,
        region: RegionId,
        header: BlockId,
        body: &[Stmt],
        continuation: Option<BlockId>,
        prior: Option<&Stmt>,
        omitted: Option<Site>,
    ) -> Result<Self> {
        let (condition, body_start) = guard(expr, body, region);
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
                Some((
                    site,
                    Simple::Assign {
                        local: *local,
                        value: Expression::value(expr.ir, *value, &expr.ir.locals[*local].ty),
                        declaration: false,
                    },
                ))
            });
        let (initializer_site, initializer) = match initializer {
            Some((site, initializer)) => (Some(site), Some(initializer)),
            None => (None, None),
        };
        Ok(Self {
            header: LoopHeader::For {
                initializer,
                condition,
                update: Box::new(Simple::Scheduled(
                    expr.ir.blocks[step.block].statements[step.statement].clone(),
                )),
            },
            body_start,
            initializer: initializer_site,
            update: Some((step.block, step.statement)),
        })
    }
}

fn guard(expr: &ExprPlanner<'_, '_>, body: &[Stmt], region: RegionId) -> (Expression, usize) {
    if let Some(condition) = updated_guard(expr, body, region) {
        return (condition, 2);
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
            return (Expression::predicate(condition, true), index + 1);
        }
        if then_body.is_empty() && exits(else_body) {
            return (Expression::predicate(condition, false), index + 1);
        }
    }
    (Expression::Boolean(true), 0)
}

/// The condition owns this exact leading assignment. A continue already jumps
/// to it, unlike moving an arbitrary trailing update into a loop header.
fn updated_guard(
    expr: &ExprPlanner<'_, '_>,
    body: &[Stmt],
    region: RegionId,
) -> Option<Expression> {
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
    Some(Expression::Compare {
        op,
        left: Box::new(Expression::Update { local, increment }),
        right: Box::new(Expression::Value(ValueUse::Reference(condition.right))),
    })
}

#[cfg(test)]
#[path = "../../../tests/unit/source_loop_controls.rs"]
mod tests;
