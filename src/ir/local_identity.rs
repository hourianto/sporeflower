//! JVM frame slots supply storage affinities, not mandatory source identities.
//! A slot may be reused, and stack values can outlive the slot they came from.
//! Dependency and interference checks must still approve every storage merge.
use super::{LocalId, MethodIr, ValueKind, lift::RawIr};
use crate::index::{BlockId, IdVec, ValueId};
use std::collections::BTreeSet;

pub(super) struct SourceSlots(IdVec<LocalId, Option<u16>>);

impl SourceSlots {
    pub(super) fn new(ir: &MethodIr) -> Self {
        let mut slots: IdVec<LocalId, _> = vec![None; ir.locals.len()].into();
        let mut ambiguous = BTreeSet::new();
        for value in &ir.values {
            let Some(local) = value.local else { continue };
            let slot = match value.kind {
                ValueKind::Phi {
                    slot: Some(slot), ..
                }
                | ValueKind::Parameter { slot, .. } => slot,
                _ => continue,
            };
            if slots[local].is_some_and(|previous| previous != slot) {
                ambiguous.insert(local);
            }
            slots[local] = Some(slot);
        }
        for local in ambiguous {
            slots[local] = None;
        }
        Self(slots)
    }

    pub(super) fn same(&self, left: LocalId, right: LocalId) -> bool {
        self.slot(left)
            .is_some_and(|slot| self.slot(right) == Some(slot))
    }

    pub(super) fn slot(&self, local: LocalId) -> Option<u16> {
        self.0.get(local).copied().flatten()
    }
}

pub(super) fn incoming_local(raw: &RawIr, header: BlockId, value: ValueId, slot: u16) -> bool {
    matches!(raw.ir.values[value].kind,
        ValueKind::Phi {
            slot: Some(source), ..
        }
        | ValueKind::Parameter { slot: source, .. }
            if source == slot)
        || raw.local_accesses.iter().any(|access| {
            access.value == value && access.slot == slot && access.pc <= raw.ir.blocks[header].pc
        })
}
