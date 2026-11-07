//! Compare cleanup completion independently of source labels and duplicated
//! return blocks. Only tail transfers may be factored: an early exit followed
//! by another cleanup operation is never erased.
use crate::{ir::MethodIr, structure::Stmt};
use std::collections::BTreeMap;

pub(super) type Bindings = BTreeMap<usize, Stmt>;

/// An inherited continuation may skip no scheduled operation. In particular,
/// reaching the same eventual return after another close is a different exit.
pub(super) fn transparent_follow(
    ir: &MethodIr,
    body: &[Stmt],
    following: Option<&Stmt>,
    bindings: &Bindings,
) -> Option<Stmt> {
    for statement in body {
        if let Some(scheduled) = statement.scheduled(ir) {
            if scheduled.statements.is_empty() {
                continue;
            }
            return None;
        }
        return follow(std::slice::from_ref(statement), bindings);
    }
    following.cloned()
}

/// A common normal continuation can be spelled inside several catch/if arms.
/// Keep it only when all such paths reach the same transfer; loop and state
/// regions remain opaque here rather than guessing their completion behavior.
pub(super) fn completion(
    ir: &MethodIr,
    body: &[Stmt],
    following: Option<&Stmt>,
    bindings: &Bindings,
) -> Option<Stmt> {
    let mut result = following.cloned();
    for statement in body.iter().rev() {
        result = match statement {
            Stmt::Basic(_) | Stmt::Statements { .. } => result,
            Stmt::Return(_) | Stmt::Break { .. } | Stmt::Continue { .. } => {
                follow(std::slice::from_ref(statement), bindings)
            }
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                let left = completion(ir, then_body, result.as_ref(), bindings)?;
                let right = completion(ir, else_body, result.as_ref(), bindings)?;
                super::equivalent::same_exit(ir, &left, &right).then_some(left)
            }
            Stmt::Try { body, catches } => {
                let exit = completion(ir, body, result.as_ref(), bindings)?;
                for catch in catches {
                    let other = completion(ir, &catch.body, result.as_ref(), bindings)?;
                    if !super::equivalent::same_exit(ir, &exit, &other) {
                        return None;
                    }
                }
                Some(exit)
            }
            Stmt::Label { region, body } => {
                let mut bindings = bindings.clone();
                if let Some(exit) = &result {
                    bindings.insert(*region, exit.clone());
                }
                completion(ir, body, result.as_ref(), &bindings)
            }
            _ => None,
        };
    }
    result
}

pub(super) fn follow(body: &[Stmt], bindings: &Bindings) -> Option<Stmt> {
    let first = body.first()?;
    match first {
        Stmt::Return(_) | Stmt::Continue { .. } => Some(first.clone()),
        Stmt::Break { .. } => {
            let mut current = first;
            let mut remaining = bindings.len() + 1;
            while let Stmt::Break { target } = current {
                let Some(next) = bindings.get(target) else {
                    return Some(current.clone());
                };
                if remaining == 0 {
                    return None;
                }
                remaining -= 1;
                current = next;
            }
            Some(current.clone())
        }
        _ => None,
    }
}

pub(super) fn normalize(
    ir: &MethodIr,
    body: &[Stmt],
    expected: &Stmt,
    bindings: &Bindings,
) -> Option<Vec<Stmt>> {
    sequence(ir, body, expected, bindings, true)
}

fn sequence(
    ir: &MethodIr,
    body: &[Stmt],
    expected: &Stmt,
    bindings: &Bindings,
    tail: bool,
) -> Option<Vec<Stmt>> {
    let mut end = body.len();
    if let Some(exit) = body
        .last()
        .and_then(|last| follow(std::slice::from_ref(last), bindings))
    {
        if !tail || !super::equivalent::same_exit(ir, &exit, expected) {
            return None;
        }
        end -= 1;
    }
    let mut result = Vec::new();
    for (index, statement) in body[..end].iter().enumerate() {
        let at_tail = tail && index + 1 == end;
        let item = match statement {
            Stmt::Return(_) | Stmt::Break { .. } | Stmt::Continue { .. } => return None,
            Stmt::If {
                condition,
                then_body,
                else_body,
            } => Stmt::If {
                condition: condition.clone(),
                then_body: sequence(ir, then_body, expected, bindings, at_tail)?,
                else_body: sequence(ir, else_body, expected, bindings, at_tail)?,
            },
            Stmt::Try { body, catches } => Stmt::Try {
                body: sequence(ir, body, expected, bindings, at_tail)?,
                catches: catches
                    .iter()
                    .map(|catch| {
                        Some(crate::structure::Catch {
                            body: sequence(ir, &catch.body, expected, bindings, at_tail)?,
                            ..catch.clone()
                        })
                    })
                    .collect::<Option<_>>()?,
            },
            Stmt::Label { region, body } if at_tail => {
                let mut bindings = bindings.clone();
                bindings.insert(*region, expected.clone());
                let body = sequence(ir, body, expected, &bindings, true)?;
                if super::super::contains_break(&body, *region) {
                    return None;
                }
                result.extend(body);
                continue;
            }
            // Internal loop/switch exits retain their identities. Their trees
            // are matched structurally and may not acquire a pending return.
            other => other.clone(),
        };
        result.push(item);
    }
    Some(result)
}
