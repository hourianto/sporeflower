//! Ordered operations of reconstructed expressions before local scheduling.
//!
//! Object creation and array initialization begin before their operands; their
//! completed values follow the operands. A recovered lazy choice is one opaque
//! region after its condition, because its arm traces were checked separately.
//! This is the identity/order projection of the same evaluation represented by
//! `local_effects` as typed reads, writes, throws and alternative paths.
use super::lift::{Event, EventKind, RawIr};
use super::{ValueId, ValueKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Operation {
    Allocate(u32),
    Complete(ValueId),
    ReadTarget(ValueId),
}

pub(super) fn has_effect(raw: &RawIr, value: ValueId) -> bool {
    raw.conditional_regions.contains_key(&value)
        || raw.ir.operation_is_observable(value)
            && !matches!(raw.ir.values[value].kind, ValueKind::Allocation { .. })
}

/// The operation at an original event. Initializer recovery retains both its
/// allocation event and its completed-value event, just as NEW/INIT already do.
pub(super) fn original(raw: &RawIr, event: &Event) -> Option<Operation> {
    match event.kind {
        EventKind::ReadTarget(value) => Some(Operation::ReadTarget(value)),
        EventKind::Allocate(_) => Some(Operation::Allocate(event.pc)),
        EventKind::Define(value) => match raw.ir.values[value].kind {
            ValueKind::Allocation { .. } => Some(Operation::Allocate(event.pc)),
            _ if has_effect(raw, value) => Some(Operation::Complete(value)),
            _ => None,
        },
        _ => None,
    }
}

/// Enumerate source operations, allowing the caller to stop at a local read or
/// a definition outside an owned region. `enter` is called before the allocation
/// boundary and can deduplicate first-use captures in a lazy expression arm.
pub(super) fn expression(
    raw: &RawIr,
    value: ValueId,
    root: bool,
    mut enter: impl FnMut(ValueId, bool) -> bool,
    mut emit: impl FnMut(Operation),
) {
    let mut pending = vec![(value, root, 0)];
    while let Some((value, root, complete)) = pending.pop() {
        if complete == 2 {
            emit(Operation::ReadTarget(value));
            continue;
        }
        if complete == 1 {
            if has_effect(raw, value) {
                emit(Operation::Complete(value));
            }
            continue;
        }
        if !enter(value, root) {
            continue;
        }
        let kind = &raw.ir.values[value].kind;
        if matches!(
            kind,
            ValueKind::Phi { .. }
                | ValueKind::Parameter { .. }
                | ValueKind::CaughtException { .. }
                | ValueKind::Allocation { .. }
        ) {
            continue;
        }
        if let ValueKind::New { allocation_pc, .. }
        | ValueKind::ArrayInitializer { allocation_pc, .. } = kind
        {
            emit(Operation::Allocate(*allocation_pc));
        }
        pending.push((value, root, 1));
        let start = pending.len();
        if let ValueKind::CompoundUpdate { target, right, .. } = kind {
            // Build in reverse stack order: target, saved-value read, RHS, write.
            pending.push((*right, false, 0));
            pending.push((value, root, 2));
            let operands = pending.len();
            target.for_each_operand(|operand| pending.push((operand, false, 0)));
            pending[operands..].reverse();
            continue;
        }
        if raw.conditional_regions.contains_key(&value) {
            let ValueKind::Select { condition, .. } = kind else {
                unreachable!()
            };
            pending.push((condition.left, false, 0));
            pending.push((condition.right, false, 0));
        } else {
            kind.for_each_operand(|operand| pending.push((operand, false, 0)));
        }
        pending[start..].reverse();
    }
}
