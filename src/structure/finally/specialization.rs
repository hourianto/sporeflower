//! A copied cleanup may omit a branch whose local content is already known at
//! that particular normal exit. Handler-entry types and SSA definitions alone
//! do not prove this: the query uses current physical-local contents.
use super::Analysis;
use crate::ir::{CompareOp, Condition, Literal, LocalId, MethodIr, ValueId, ValueKind};
use std::collections::BTreeSet;

pub(super) struct Decision {
    pub taken: bool,
    pub reads: BTreeSet<LocalId>,
}

enum Operand<'a> {
    Local(LocalId),
    Literal(&'a Literal),
}

fn operand(ir: &MethodIr, mut value: ValueId) -> Option<Operand<'_>> {
    loop {
        if let Some(local) = ir.values[value].local {
            return Some(Operand::Local(local));
        }
        match &ir.values[value].kind {
            ValueKind::Literal(literal @ (Literal::Null | Literal::Int(_))) => {
                return Some(Operand::Literal(literal));
            }
            ValueKind::Alias(source) => value = *source,
            // Even a repeatable field, cast, array read, or arithmetic guard
            // could throw or observe changed state. Only reads and constants
            // can disappear from the copied source path.
            _ => return None,
        }
    }
}

pub(super) fn condition(
    analysis: &Analysis<'_>,
    point: (crate::ir::BlockId, usize),
    condition: &Condition,
) -> Option<Decision> {
    let ir = analysis.ir;
    if !matches!(condition.op, CompareOp::Eq | CompareOp::Ne) {
        return None;
    }
    let mut reads = BTreeSet::new();
    let equal = match (operand(ir, condition.left)?, operand(ir, condition.right)?) {
        (Operand::Literal(left), Operand::Literal(right)) => match (left, right) {
            (Literal::Null, Literal::Null) => true,
            (Literal::Int(left), Literal::Int(right)) => left == right,
            _ => return None,
        },
        (Operand::Local(local), Operand::Literal(literal))
        | (Operand::Literal(literal), Operand::Local(local)) => {
            let equal = if analysis
                .available()
                .reads_literal(point.0, point.1, local, literal)
            {
                true
            } else if matches!(literal, Literal::Null)
                && analysis.available().reads_non_null(point.0, point.1, local)
            {
                false
            } else {
                return None;
            };
            reads.insert(local);
            equal
        }
        (Operand::Local(_), Operand::Local(_)) => return None,
    };
    Some(Decision {
        taken: equal == (condition.op == CompareOp::Eq),
        reads,
    })
}
