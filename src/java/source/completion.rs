//! Java normal completion follows lexical transfer ownership, including
//! compound abrupt arms. The same fact governs sequence tails and case sharing.
use super::{Body, Expression, LoopHeader, Statement};

pub(super) trait Truth {
    fn always_true(&self) -> bool;
}
impl Truth for Expression {
    fn always_true(&self) -> bool {
        matches!(self, Self::Boolean(true))
    }
}
impl Truth for crate::java::syntax::Expr {
    fn always_true(&self) -> bool {
        self.boolean_constant() == Some(true)
    }
}

pub(super) fn can_complete<E: Truth, S, V, T>(body: &[Statement<E, S, V, T>]) -> bool {
    body.last().is_none_or(statement)
}

pub(super) fn breaks_loop<E: Truth, S, V, T>(
    body: &[Statement<E, S, V, T>],
    label: &Option<String>,
) -> bool {
    transfer(body, label, true, false, true)
}

/// Syntax still needs an owning loop when a finally overrides the transfer.
pub(super) fn targets_loop<E: Truth, S, V, T>(
    body: &[Statement<E, S, V, T>],
    label: &Option<String>,
    continuing: bool,
) -> bool {
    transfers_to(body, label, true, continuing)
}

/// Query lexical transfers for an explicit enclosing target. Bare transfers
/// belong only to the nearest compatible construct selected by the caller.
pub(super) fn transfers_to<E: Truth, S, V, T>(
    body: &[Statement<E, S, V, T>],
    label: &Option<String>,
    unlabelled: bool,
    continuing: bool,
) -> bool {
    transfer(body, label, unlabelled, continuing, false)
}

/// Compose one proved cleanup with its protected source body. A bare return
/// evaluates nothing: normal completion can run the cleanup first and return
/// afterward, with any abrupt cleanup still overriding that return. No value
/// expression or other continuation may cross the catch/cleanup boundaries.
pub(super) fn with_cleanup(mut body: Body, cleanup: Body) -> Body {
    let returns_void = matches!(
        body.as_slice(),
        [
            Statement::Try { cleanup: None, .. },
            Statement::Return(None)
        ]
    );
    if returns_void {
        body.pop();
    }
    let statement = if matches!(body.as_slice(), [Statement::Try { cleanup: None, .. }]) {
        let Statement::Try { body, catches, .. } = body.pop().unwrap() else {
            unreachable!()
        };
        Statement::Try {
            body,
            catches,
            cleanup: Some(cleanup),
        }
    } else {
        Statement::Try {
            body,
            catches: Vec::new(),
            cleanup: Some(cleanup),
        }
    };
    let mut result = vec![statement];
    if returns_void && can_complete(&result) {
        result.push(Statement::Return(None));
    }
    result
}

fn statement<E: Truth, S, V, T>(value: &Statement<E, S, V, T>) -> bool {
    match value {
        Statement::Return(_)
        | Statement::Throw(_)
        | Statement::Break(_)
        | Statement::Continue(_) => false,
        Statement::If { yes, no, .. } => can_complete(yes) || can_complete(no),
        Statement::Block {
            label,
            body: nested,
        } => {
            can_complete(nested)
                || label
                    .as_ref()
                    .is_some_and(|_| transfer(nested, label, false, false, true))
        }
        Statement::Synchronized { body: nested, .. } => can_complete(nested),
        Statement::Try {
            body: nested,
            catches,
            cleanup,
        } => {
            (can_complete(nested) || catches.iter().any(|catch| can_complete(&catch.body)))
                && cleanup.as_ref().is_none_or(|cleanup| can_complete(cleanup))
        }
        Statement::Loop {
            label,
            header,
            body: nested,
        } => {
            if breaks_loop(nested, label) {
                return true;
            }
            let condition = match header {
                LoopHeader::While(condition)
                | LoopHeader::DoWhile(condition)
                | LoopHeader::For { condition, .. } => condition,
            };
            if condition.always_true() {
                return false;
            }
            !matches!(header, LoopHeader::DoWhile(_))
                || can_complete(nested)
                || transfer(nested, label, true, true, true)
        }
        Statement::Switch { label, cases, .. } => {
            !cases.iter().any(|case| case.default)
                || cases.iter().any(|case| breaks_loop(&case.body, label))
                || cases.last().is_none_or(|case| can_complete(&case.body))
        }
        Statement::Simple(_) => true,
    }
}

fn transfer<E: Truth, S, V, T>(
    body: &[Statement<E, S, V, T>],
    label: &Option<String>,
    unlabelled: bool,
    continuing: bool,
    reachable_only: bool,
) -> bool {
    let mut pending = vec![(body, unlabelled)];
    while let Some((body, unlabelled)) = pending.pop() {
        for statement in body {
            match statement {
                Statement::Break(target) if !continuing => {
                    if target.is_none() && unlabelled || target.is_some() && target == label {
                        return true;
                    }
                }
                Statement::Continue(target) if continuing => {
                    if target.is_none() && unlabelled || target.is_some() && target == label {
                        return true;
                    }
                }
                Statement::If { yes, no, .. } => {
                    pending.push((yes, unlabelled));
                    pending.push((no, unlabelled));
                }
                Statement::Loop { body, .. } => pending.push((body, false)),
                Statement::Switch { cases, .. } => {
                    pending.extend(
                        cases
                            .iter()
                            .map(|case| (case.body.as_slice(), unlabelled && continuing)),
                    );
                }
                Statement::Block { body, .. } | Statement::Synchronized { body, .. } => {
                    pending.push((body, unlabelled))
                }
                Statement::Try {
                    body: nested,
                    catches,
                    cleanup,
                } => {
                    if !reachable_only
                        || cleanup.as_ref().is_none_or(|cleanup| can_complete(cleanup))
                    {
                        pending.push((nested.as_slice(), unlabelled));
                        pending.extend(
                            catches
                                .iter()
                                .map(|catch| (catch.body.as_slice(), unlabelled)),
                        );
                    }
                    if let Some(cleanup) = cleanup {
                        pending.push((cleanup, unlabelled));
                    }
                }
                _ => (),
            }
        }
    }
    false
}

#[cfg(test)]
#[path = "../../../tests/unit/source_completion.rs"]
mod tests;
