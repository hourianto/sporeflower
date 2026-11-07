//! Consume exact scheduled initializer and update sites into a source loop.
//! Post-test spelling uses final declaration visibility; no omission plan or
//! guessed binding scope survives into the writer.
use super::{Body, Expression, LoopHeader, Simple, Statement as Source};
use crate::java::{expr::ExprPlanner, source_updates};
use crate::{
    Result,
    classfile::JvmType,
    index::IdVec,
    ir::{BlockId, Literal, LocalId, MethodIr, Statement, Terminator, ValueKind, ValueUse},
    structure::{Predicate, RegionId, Stmt},
};

mod repetition;
pub(super) use repetition::{accesses as control_accesses, expand as expand_finite};

pub(super) type Site = (BlockId, usize);

/// Choose the spelling of this loop as its already-lowered children arrive.
/// The update has its final owner before the body tail is considered.
pub(super) fn construct(label: Option<String>, mut header: LoopHeader, mut body: Body) -> Body {
    let mut outward = None;
    if matches!(
        &header,
        LoopHeader::While(Expression::Boolean(true))
            | LoopHeader::For {
                condition: Expression::Boolean(true),
                ..
            }
    ) && let Some(Source::If { yes, no, .. }) = body.first()
        && (no.is_empty() && matches!(yes.as_slice(), [Source::Return(_)])
            || yes.is_empty() && matches!(no.as_slice(), [Source::Return(_)]))
        && !super::completion::breaks_loop(&body[1..], &label)
    {
        let Source::If {
            condition,
            mut yes,
            mut no,
        } = body.remove(0)
        else {
            unreachable!()
        };
        let (condition, exit) = if no.is_empty() {
            (condition.negate(), yes.pop().unwrap())
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
        outward = Some(exit);
    }
    // A tail filter around one action reads as that action's positive
    // condition. Keep larger work and compound tests in their existing
    // guard form, without adding nesting around an arbitrary body suffix.
    let end = body.len();
    if end >= 2
        && matches!(
            &body[end - 1],
            Source::Simple(
                Simple::Scheduled(_)
                    | Simple::CatchBind { .. }
                    | Simple::Dispatch { .. }
                    | Simple::Assign {
                        declaration: false,
                        ..
                    }
            )
        )
        && matches!(&body[end - 2], Source::If {
                condition: Expression::Predicate { predicate: Predicate::Atom { .. }, .. },
                yes, no,
            } if matches!(yes.as_slice(), [Source::Continue(None)]) && no.is_empty())
    {
        let action = body.pop().unwrap();
        let Source::If { condition, .. } = body.pop().unwrap() else {
            unreachable!()
        };
        body.push(Source::If {
            condition: condition.negate(),
            yes: vec![action],
            no: Vec::new(),
        });
    }
    let mut result = vec![Source::Loop {
        label,
        header,
        body,
    }];
    result.extend(outward);
    result
}

/// Final scopes certify private control storage and post-test visibility.
/// Method-wide unique names also permit a single execution's body to flatten.
pub(in crate::java) fn finish(
    ir: &MethodIr,
    statement: Source,
    accesses: &IdVec<LocalId, usize>,
) -> Body {
    let Source::Loop {
        label,
        mut header,
        mut body,
    } = statement
    else {
        return vec![statement];
    };
    if let Some(once) = trivial_repetition(ir, &header, accesses)
        && (!once
            || !super::completion::targets_loop(&body, &label, false)
                && !super::completion::targets_loop(&body, &label, true))
    {
        return if once { body } else { Vec::new() };
    }
    if let Some((condition, end)) = post_test(&header, &body, &label)
        && post_test_eligible(ir, &condition, &body[..end], &label)
    {
        header = LoopHeader::DoWhile(condition);
        body.truncate(end);
    }
    vec![Source::Loop {
        label,
        header,
        body,
    }]
}

/// The existing access scan proves that this private counter belongs only to
/// its initializer, guard and update. Pure integer controls can then certify
/// zero or one executions without substituting constants into the loop body.
fn trivial_repetition(
    ir: &MethodIr,
    header: &LoopHeader,
    accesses: &IdVec<LocalId, usize>,
) -> Option<bool> {
    let LoopHeader::For {
        initializer:
            Some(Simple::Assign {
                local,
                value: Expression::Value(initial),
                declaration: true,
            }),
        condition:
            Expression::Predicate {
                predicate: Predicate::Atom { head, negated },
                negated: inverted,
            },
        update,
    } = header
    else {
        return None;
    };
    if accesses[*local] != 3 {
        return None;
    }
    let start = integer(ir, *initial)?;
    let Terminator::Branch { condition, .. } = &ir.blocks[*head].terminator else {
        return None;
    };
    let limit = integer(ir, ValueUse::Reference(condition.right))?;
    let Simple::Scheduled(Statement::Assign {
        local: updated,
        value,
    }) = update.as_ref()
    else {
        return None;
    };
    if updated != local || ir.values[condition.left].local != Some(*local) {
        return None;
    }
    let next = start.wrapping_add(source_updates::integer_step(ir, *local, *value)?);
    let test = |value: i32| condition.op.test(Some(value.cmp(&limit))) ^ negated ^ inverted;
    let once = test(start);
    (!once || !test(next)).then_some(once)
}

fn integer(ir: &MethodIr, usage: ValueUse) -> Option<i32> {
    let value = &ir.values[usage.value()];
    match value.kind {
        ValueKind::Literal(Literal::Int(number)) if usage.local(ir).is_none() => Some(number),
        _ => None,
    }
}

fn post_test(
    header: &LoopHeader,
    body: &[Source],
    label: &Option<String>,
) -> Option<(Expression, usize)> {
    if !matches!(header, LoopHeader::While(Expression::Boolean(true))) {
        return None;
    }
    // The terminal decision owns the repeat/exit choice, whether recovery
    // spelled its repeat as continue or left it as ordinary fallthrough.
    match body {
        [.., Source::If { condition, yes, no }, Source::Break(target)]
            if (target.is_none() || target == label)
                && no.is_empty()
                && matches!(yes.as_slice(), [Source::Continue(None)]) =>
        {
            Some((condition.clone(), body.len() - 2))
        }
        [.., Source::If { condition, yes, no }]
            if no.is_empty()
                && matches!(yes.as_slice(), [Source::Break(target)] if target.is_none() || target == label) =>
        {
            Some((condition.clone().negate(), body.len() - 1))
        }
        _ => None,
    }
}

fn post_test_eligible(
    ir: &MethodIr,
    condition: &Expression,
    body: &[Source],
    label: &Option<String>,
) -> bool {
    let locals = super::access::expression(ir, condition);
    // Placement has already hoisted any binding shared with a nested scope.
    // Only declarations in this body's own scope can hide a terminal operand.
    !super::completion::targets_loop(body, label, true)
        && !body.iter().any(|statement| {
            matches!(statement,
            Source::Simple(Simple::Declare { local, .. }
                | Simple::Assign { local, declaration: true, .. }) if locals.contains(local))
        })
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
