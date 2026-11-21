//! Source exception scopes need not include operations which cannot throw.
//! Hoisting their initial prefix exposes actual SSA initialization to Java's
//! definite-assignment rules without introducing default values.
use super::Stmt;
use crate::ir::{MethodIr, Terminator};

pub(super) fn normalize(ir: &MethodIr, body: Vec<Stmt>) -> Vec<Stmt> {
    let mut result = Vec::new();
    for statement in body {
        let mut statement = statement.map_bodies(|body| normalize(ir, body));
        let finally = matches!(statement, Stmt::TryFinally { .. });
        if let Stmt::Try { body, .. } | Stmt::TryFinally { body, .. } = &mut statement {
            let prefix = body
                .iter()
                .take_while(|statement| {
                    // A finally also runs on outward transfers. Only an
                    // unconditional scheduled prefix can move before it.
                    (!finally || statement.scheduled(ir).is_some())
                        && nonthrowing(ir, statement)
                        && super::can_complete(std::slice::from_ref(*statement))
                })
                .count();
            result.extend(body.drain(..prefix));
        }
        // A catch can protect the inner handlers, but none of the exceptions
        // already caught by the inner try can escape a nonthrowing handler.
        if let Stmt::Try { body, catches } = &mut statement
            && let Some((Stmt::Try { catches: inner, .. }, tail)) = body.split_first()
            && tail.iter().all(|s| nonthrowing(ir, s))
            && catches.iter().all(|outer| {
                inner.iter().any(|catch| {
                    catch.catch_type == outer.catch_type
                        || crate::cfg::dispatch::is_catch_all(catch.catch_type.as_deref())
                })
            })
            && inner
                .iter()
                .all(|catch| catch.body.iter().all(|s| nonthrowing(ir, s)))
        {
            result.append(body);
            continue;
        }
        // A successful probe returns. An unused exception can fall through to
        // the next probe instead of nesting its entire try in this catch.
        if let Stmt::Try { body, catches } = &mut statement
            && matches!(body.last(), Some(Stmt::Return(_)))
            && let [catch] = catches.as_mut_slice()
            && ir.blocks[catch.handler]
                .caught_exception
                .is_some_and(|value| ir.values[value].local.is_none())
            && let Some(next) = catch
                .body
                .iter()
                .position(|s| matches!(s, Stmt::Try { .. }))
        {
            let continuation = catch.body.split_off(next);
            result.push(statement);
            result.extend(continuation);
            continue;
        }
        result.push(statement);
    }
    result
}

fn nonthrowing(ir: &MethodIr, statement: &Stmt) -> bool {
    let supported = match statement {
        Stmt::Basic(_)
        | Stmt::Statements { .. }
        | Stmt::Label { .. }
        | Stmt::Break { .. }
        | Stmt::Continue { .. } => true,
        Stmt::If { condition, .. } => condition
            .heads()
            .into_iter()
            .all(|head| matches!(ir.blocks[head].terminator, Terminator::Branch { .. })),
        Stmt::Switch { head, .. } => {
            matches!(ir.blocks[*head].terminator, Terminator::Switch { .. })
        }
        _ => false,
    };
    let mut throws = false;
    if supported {
        statement.operations(ir, &mut |operation| throws |= operation.may_throw(ir));
    }
    supported
        && !throws
        && statement
            .bodies()
            .flatten()
            .all(|child| nonthrowing(ir, child))
}
