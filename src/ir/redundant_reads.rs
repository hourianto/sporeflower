//! Remove unused field evaluations whose only possible effects already occurred.
//!
//! A successful earlier read of the same symbolic field and immutable receiver
//! proves resolution, initialization and null checks for the rest of the block.
//! The field's value need not be stable: we discard only results nobody uses.
//! Volatile reads retain their synchronization effect even when discarded.

use super::lift::{EventKind, RawIr};
use super::{FieldAccessFlags, ValueKind};
use crate::classfile::ClassFile;
use crate::index::{IdVec, ValueId};
use std::collections::{HashMap, HashSet};

pub(super) fn eliminate(
    raw: &mut RawIr,
    class: &ClassFile,
    field_access_flags: Option<FieldAccessFlags<'_>>,
) {
    if raw
        .ir
        .values
        .iter()
        .filter(|value| matches!(value.kind, ValueKind::Field { .. }))
        .take(2)
        .count()
        < 2
    {
        return;
    }
    let mut used: IdVec<ValueId, _> = vec![false; raw.ir.values.len()].into();
    for value in &raw.ir.values {
        value.kind.for_each_operand(|operand| used[operand] = true);
    }
    for events in &raw.events {
        for event in events {
            if let EventKind::Action(action) = &event.kind {
                for operand in action.operands() {
                    used[operand] = true;
                }
            }
        }
    }
    for block in &raw.ir.blocks {
        for operand in block.terminator.operands() {
            used[operand] = true;
        }
    }

    let mut flags = HashMap::new();
    let mut removed = HashSet::new();
    for events in &raw.events {
        let mut successful = HashSet::new();
        for event in events {
            let EventKind::Define(value) = event.kind else {
                continue;
            };
            let ValueKind::Field { field, receiver } = &raw.ir.values[value].kind else {
                continue;
            };
            if successful.insert((field, receiver)) || used[value] {
                continue;
            }
            let nonvolatile = *flags.entry(field).or_insert_with(|| {
                field_access_flags
                    .and_then(|query| query(field))
                    .or_else(|| {
                        (field.owner == class.name)
                            .then(|| {
                                class.fields.iter().find(|declared| {
                                    declared.name == field.name
                                        && declared.descriptor == field.descriptor
                                })
                            })
                            .flatten()
                            .map(|declared| declared.access_flags)
                    })
                    .is_some_and(|access| access & 0x0040 == 0)
            });
            if nonvolatile {
                removed.insert(value);
            }
        }
    }
    if removed.is_empty() {
        return;
    }

    // Keep operand producers: a discarded read may still have evaluated an
    // observable receiver. Only its own successful, nonvolatile access is gone.
    for events in &mut raw.events {
        events.retain(
            |event| !matches!(event.kind, EventKind::Define(value) if removed.contains(&value)),
        );
    }
    for value in removed {
        raw.ir.values[value].kind = ValueKind::Phi {
            inputs: vec![],
            slot: None,
        };
    }
}

#[cfg(test)]
#[path = "../../tests/unit/redundant_reads.rs"]
mod tests;
