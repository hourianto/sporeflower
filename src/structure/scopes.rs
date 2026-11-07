//! Source exception scopes need not include operations which cannot throw.
//! Hoisting their initial prefix exposes actual SSA initialization to Java's
//! definite-assignment rules without introducing default values.
use super::Stmt;
use crate::ir::{MethodIr, Terminator};

pub(super) fn normalize(ir: &MethodIr, body: Vec<Stmt>) -> Vec<Stmt> {
    let mut result = Vec::new();
    for statement in body {
        let statement = statement.map_bodies(|body| normalize(ir, body));
        if let Stmt::Try { mut body, catches } = statement {
            let prefix = body
                .iter()
                .take_while(|statement| {
                    nonthrowing(ir, statement)
                        && super::can_complete(std::slice::from_ref(*statement))
                })
                .count();
            let remaining = body.split_off(prefix);
            result.extend(body);
            result.push(Stmt::Try {
                body: remaining,
                catches,
            });
        } else {
            result.push(statement);
        }
    }
    result
}

fn nonthrowing(ir: &MethodIr, statement: &Stmt) -> bool {
    match statement {
        Stmt::Basic(_) | Stmt::Statements { .. } => statement.scheduled(ir).unwrap()
            .statements
            .iter()
            .all(|statement| !super::statement_may_throw(ir, statement)),
        Stmt::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            condition.heads().into_iter().all(|head| matches!(&ir.blocks[head].terminator, Terminator::Branch { condition, .. }
                if !super::values_may_throw(ir, vec![(condition.left, false), (condition.right, false)])))
                && then_body
                    .iter()
                    .chain(else_body)
                    .all(|statement| nonthrowing(ir, statement))
        }
        Stmt::Switch { head, cases, .. } => {
            matches!(&ir.blocks[*head].terminator, Terminator::Switch { value, .. } if !super::values_may_throw(ir, vec![(*value, false)]))
                && cases
                    .iter()
                    .all(|case| case.body.iter().all(|statement| nonthrowing(ir, statement)))
        }
        Stmt::Label { body, .. } => body.iter().all(|statement| nonthrowing(ir, statement)),
        Stmt::Break { .. } | Stmt::Continue { .. } => true,
        _ => false,
    }
}
