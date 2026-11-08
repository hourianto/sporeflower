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
