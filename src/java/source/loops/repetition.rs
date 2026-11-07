//! A finite counter can select exit work as well as bound the loop body.
//! Keep that exhaustion continuation on its original repeat paths when the
//! counter is private and proves at most one useful iteration.
use super::{Body, Expression, LoopHeader, Simple, Source, integer};
use crate::{
    index::IdVec,
    ir::{LocalId, MethodIr, Statement, Terminator, ValueUse},
    java::{local_flow, source::completion, source_updates},
    structure::Predicate,
};

struct Expansion {
    initializer: Option<usize>,
    enter: bool,
    exhausted_arm: bool,
}

/// This proof is deliberately about every scheduled use, including handlers
/// and source paths that a later pass may discard. Declaration placement still
/// scans the resulting source once, after these controls have been consumed.
pub(in crate::java::source) fn accesses(ir: &MethodIr) -> IdVec<LocalId, usize> {
    let mut counts: IdVec<LocalId, usize> = vec![0; ir.locals.len()].into();
    for block in &ir.blocks {
        for statement in &block.statements {
            for local in local_flow::statement_accesses(ir, statement) {
                counts[local] += 1;
            }
        }
        let locals: std::collections::BTreeSet<_> = block
            .terminator
            .operands()
            .into_iter()
            .flat_map(|value| local_flow::value_accesses(ir, ValueUse::Reference(value)))
            .collect();
        for local in locals {
            counts[local] += 1;
        }
    }
    counts
}

pub(in crate::java::source) fn expand(
    ir: &MethodIr,
    body: &mut Body,
    accesses: &IdVec<LocalId, usize>,
) {
    let mut index = 0;
    while index < body.len() {
        let Some(plan) = recognize(ir, body, index, accesses) else {
            index += 1;
            continue;
        };
        let Source::Loop {
            label,
            body: mut nested,
            ..
        } = body.remove(index)
        else {
            unreachable!()
        };
        if let Some(initializer) = plan.initializer {
            body.remove(initializer);
            index -= 1;
        }
        let Source::If { yes, no, .. } = nested.remove(0) else {
            unreachable!()
        };
        let mut exhausted = if plan.exhausted_arm { yes } else { no };
        exhausted.pop();
        let replacement = if plan.enter {
            let mut exhausted = Some(exhausted);
            let tail = if completion::can_complete(&nested) {
                exhausted.take().unwrap()
            } else {
                Vec::new()
            };
            project(nested, tail, &label, &mut exhausted)
        } else {
            exhausted
        };
        let length = replacement.len();
        body.splice(index..index, replacement);
        if length == 1 && forward_choice(ir, body, index, accesses) {
            index = index.saturating_sub(1);
            continue;
        }
        index += length;
    }
}

/// Inlined Boolean routines often leave their selected result in private
/// storage. The existing selected-expression form owns that choice directly
/// when the next truth test is its only consumer; no call or store moves past
/// an intervening statement, and the original predicate is evaluated once.
fn forward_choice(
    ir: &MethodIr,
    body: &mut Body,
    index: usize,
    accesses: &IdVec<LocalId, usize>,
) -> bool {
    let Some(previous) = index.checked_sub(1) else {
        return false;
    };
    let Some(
        [
            Source::Simple(Simple::Assign {
                local,
                value: Expression::Boolean(initial),
                ..
            }),
            Source::If { yes, no, .. },
            Source::If {
                condition: consumer,
                ..
            },
        ],
    ) = body.get(previous..=index + 1)
    else {
        return false;
    };
    if accesses[*local] != 3 || ir.locals[*local].parameter.is_some() {
        return false;
    }
    let assigned = |body: &[Source]| match body {
        [
            Source::Simple(Simple::Assign {
                local: assigned,
                value: Expression::Boolean(value),
                ..
            }),
        ] if assigned == local => Some(*value),
        _ => None,
    };
    let (yes_value, no_value) = match (assigned(yes), assigned(no)) {
        (Some(yes), None) if no.is_empty() => (yes, *initial),
        (None, Some(no)) if yes.is_empty() => (*initial, no),
        _ => return false,
    };
    let Some(on_false) = test(ir, consumer, *local, 0) else {
        return false;
    };
    let Some(on_true) = test(ir, consumer, *local, 1) else {
        return false;
    };
    if on_false == on_true {
        return false;
    }
    let Source::If { condition, .. } = body.remove(index) else {
        unreachable!()
    };
    body.remove(previous);
    let Source::If {
        condition: consumer,
        ..
    } = &mut body[previous]
    else {
        unreachable!()
    };
    *consumer = Expression::Select {
        condition: Box::new(condition),
        yes: Box::new(Expression::Boolean(yes_value ^ on_false)),
        no: Box::new(Expression::Boolean(no_value ^ on_false)),
    };
    true
}

fn recognize(
    ir: &MethodIr,
    sequence: &[Source],
    index: usize,
    accesses: &IdVec<LocalId, usize>,
) -> Option<Expansion> {
    let Source::Loop {
        label,
        header:
            LoopHeader::For {
                initializer,
                condition: Expression::Boolean(true),
                update,
            },
        body,
    } = &sequence[index]
    else {
        return None;
    };
    let Simple::Scheduled(Statement::Assign { local, value }) = update.as_ref() else {
        return None;
    };
    if accesses[*local] != 3 {
        return None;
    }
    let step = source_updates::integer_step(ir, *local, *value)?;
    let (site, initializer) = if let Some(initializer) = initializer {
        (None, initializer)
    } else {
        let site = sequence[..index].iter().rposition(|statement| {
            matches!(statement,
            Source::Simple(Simple::Assign { local: assigned, .. }) if assigned == local)
        })?;
        let Source::Simple(initializer) = &sequence[site] else {
            unreachable!()
        };
        (Some(site), initializer)
    };
    let Simple::Assign {
        local: initialized,
        value: Expression::Value(initial),
        ..
    } = initializer
    else {
        return None;
    };
    if initialized != local || ir.locals[*local].parameter.is_some() || ir.locals[*local].is_this {
        return None;
    }
    let start = integer(ir, *initial)?;
    let Source::If { condition, yes, no } = body.first()? else {
        return None;
    };
    let exhausted_arm = match (exits(yes, label), exits(no, label)) {
        (true, false) if no.is_empty() => true,
        (false, true) if yes.is_empty() => false,
        _ => return None,
    };
    let exhausted = if exhausted_arm { yes } else { no };
    // The exhaustion suffix remains outside every protected child. Any
    // transfer inside it belongs to its existing owner, never the removed loop.
    if targets(&exhausted[..exhausted.len() - 1], label) {
        return None;
    }
    let enter = test(ir, condition, *local, start)? != exhausted_arm;
    if enter {
        if test(ir, condition, *local, start.wrapping_add(step))? != exhausted_arm {
            return None;
        }
        let rest = &body[1..];
        let mut repeats = usize::from(completion::can_complete(rest));
        if !projectable(rest, label, exhausted.len() > 1, &mut repeats, 0) || repeats > 1 {
            return None;
        }
    }
    Some(Expansion {
        initializer: site,
        enter,
        exhausted_arm,
    })
}

fn test(ir: &MethodIr, expression: &Expression, local: LocalId, value: i32) -> Option<bool> {
    let Expression::Predicate {
        predicate: Predicate::Atom { head, negated },
        negated: inverted,
    } = expression
    else {
        return None;
    };
    let Terminator::Branch { condition, .. } = &ir.blocks[*head].terminator else {
        return None;
    };
    if ir.values[condition.left].local != Some(local) {
        return None;
    }
    let limit = integer(ir, ValueUse::Reference(condition.right))?;
    Some(condition.op.test(Some(value.cmp(&limit))) ^ negated ^ inverted)
}

fn own(target: &Option<String>, label: &Option<String>) -> bool {
    target.is_none() || target == label
}

fn exits(body: &[Source], label: &Option<String>) -> bool {
    matches!(body.last(), Some(Source::Break(target)) if own(target, label))
}

fn targets(body: &[Source], label: &Option<String>) -> bool {
    completion::targets_loop(body, label, false) || completion::targets_loop(body, label, true)
}

/// A suffix can move to its sole continuing arm. If two arms would need a
/// copy, retain the original loop. Protected and nested scopes keep their
/// own transfers; this projection never routes work through a cleanup.
fn projectable(
    body: &[Source],
    label: &Option<String>,
    mut tail: bool,
    repeats: &mut usize,
    depth: usize,
) -> bool {
    if depth > 128 {
        return false;
    }
    for statement in body.iter().rev() {
        match statement {
            Source::Break(target) if own(target, label) => tail = false,
            Source::Continue(target) if own(target, label) => {
                *repeats += 1;
                tail = true;
            }
            Source::If { yes, no, .. } if targets(std::slice::from_ref(statement), label) => {
                let yes_tail = tail && completion::can_complete(yes);
                let no_tail = tail && completion::can_complete(no);
                if yes_tail && no_tail
                    || !projectable(yes, label, yes_tail, repeats, depth + 1)
                    || !projectable(no, label, no_tail, repeats, depth + 1)
                {
                    return false;
                }
                tail = true;
            }
            _ if targets(std::slice::from_ref(statement), label) => return false,
            _ => tail = true,
        }
    }
    true
}

fn project(body: Body, tail: Body, label: &Option<String>, exhausted: &mut Option<Body>) -> Body {
    let mut tail: std::collections::VecDeque<_> = tail.into();
    for statement in body.into_iter().rev() {
        match statement {
            Source::Break(target) if own(&target, label) => tail.clear(),
            Source::Continue(target) if own(&target, label) => {
                tail = exhausted.take().unwrap().into();
            }
            Source::If {
                mut condition,
                yes,
                no,
            } if targets(&yes, label) || targets(&no, label) => {
                let yes_tail = if completion::can_complete(&yes) {
                    std::mem::take(&mut tail).into()
                } else {
                    Vec::new()
                };
                let no_tail = if completion::can_complete(&no) {
                    std::mem::take(&mut tail).into()
                } else {
                    Vec::new()
                };
                let mut yes = project(yes, yes_tail, label, exhausted);
                let mut no = project(no, no_tail, label, exhausted);
                if yes.is_empty() && !no.is_empty() {
                    condition = condition.negate();
                    std::mem::swap(&mut yes, &mut no);
                }
                tail = [Source::If { condition, yes, no }].into();
            }
            statement => tail.push_front(statement),
        }
    }
    tail.into()
}
