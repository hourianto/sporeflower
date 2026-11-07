//! Cost the source that a linear block actually emits. Sharing an immutable
//! block in the source tree is safe only after recovery proves its control and
//! exception context; this module supplies the independent source-size bound.
use crate::ir::scheduled::{self, ValueUse};
use crate::ir::{Literal, MethodIr, Statement, Terminator, ValueKind};

pub(super) const BLOCK_COST_LIMIT: usize = 48;

pub(super) fn linear_cost(ir: &MethodIr, block: crate::ir::BlockId) -> Option<u16> {
    let block = &ir.blocks[block];
    if !matches!(
        block.terminator,
        Terminator::Goto(_) | Terminator::Return(_) | Terminator::Throw { .. }
    ) {
        return None;
    }
    let mut cost = usize::from(!matches!(block.terminator, Terminator::Goto(_)));
    let mut pending = Vec::new();
    for statement in &block.statements {
        if matches!(
            statement,
            Statement::Monitor { .. } | Statement::ConstructorCall { .. }
        ) {
            return None;
        }
        cost += 1;
        scheduled::for_each_statement_root(statement, |root| pending.push(root));
        if cost + pending.len() > BLOCK_COST_LIMIT {
            return None;
        }
    }
    pending.extend(
        block
            .terminator
            .operands()
            .into_iter()
            .map(ValueUse::Reference),
    );
    while let Some(usage) = pending.pop() {
        cost += 1;
        if cost > BLOCK_COST_LIMIT {
            return None;
        }
        if usage.local(ir).is_some() {
            continue;
        }
        match &ir.values[usage.value()].kind {
            // An unmaterialized phi or an uninitialized allocation has no
            // independently repeatable Java expression at this boundary.
            ValueKind::Phi { .. } | ValueKind::Allocation { .. } => return None,
            ValueKind::Literal(Literal::String(text)) => cost += text.len().div_ceil(16),
            _ => usage.for_each_child(ir, |child| {
                if pending.len() <= BLOCK_COST_LIMIT {
                    pending.push(child);
                }
            }),
        }
        if cost + pending.len() > BLOCK_COST_LIMIT {
            return None;
        }
    }
    u16::try_from(cost).ok()
}
