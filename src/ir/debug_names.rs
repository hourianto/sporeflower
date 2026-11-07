//! Attach debug names only through exact JVM slot accesses. Expression operands
//! and neighboring bytecode positions do not imply source-variable identity.
use super::lift::RawIr;
use super::{LocalId, ValueKind};
use crate::classfile::{JvmType, LocalVariable};
use crate::index::IdVec;
use std::collections::BTreeMap;

pub(super) fn assign(
    raw: &mut RawIr,
    variables: &[LocalVariable],
    original_pcs: Option<&[(u32, u32)]>,
) {
    attribute(raw, variables, original_pcs, false);
}

/// Final storage can contain several original lifetimes after safe copy
/// coalescing. Their exact accesses remain useful spelling evidence even though
/// they no longer identify one original source variable. Conflicting metadata
/// for the same access is still rejected; no neighboring slot is consulted.
pub(super) fn assign_final(
    raw: &mut RawIr,
    variables: &[LocalVariable],
    original_pcs: Option<&[(u32, u32)]>,
) {
    attribute(raw, variables, original_pcs, true);
}

fn attribute(
    raw: &mut RawIr,
    variables: &[LocalVariable],
    original_pcs: Option<&[(u32, u32)]>,
    presentation: bool,
) {
    if variables.is_empty() {
        return;
    }
    for local in &mut raw.ir.locals {
        local.name_hint = None;
    }
    let declared_types: Vec<_> = variables
        .iter()
        .map(|variable| JvmType::parse(&variable.descriptor).ok())
        .collect();
    let mut by_slot = BTreeMap::<u16, Vec<usize>>::new();
    for (index, variable) in variables.iter().enumerate() {
        if variable.length != 0 {
            by_slot.entry(variable.index).or_default().push(index);
        }
    }
    let parameter_slots: Vec<_> = raw
        .ir
        .parameters
        .iter()
        .filter_map(|&value| {
            if let ValueKind::Parameter { slot, .. } = raw.ir.values[value].kind {
                Some(slot)
            } else {
                None
            }
        })
        .collect();
    let mut candidates: IdVec<LocalId, _> =
        vec![BTreeMap::<usize, usize>::new(); raw.ir.locals.len()].into();
    let mut conflicts: IdVec<LocalId, _> = vec![false; raw.ir.locals.len()].into();
    let mut observe = |local: LocalId, slot: u16, pc: u32, after_store: Option<u32>| {
        if raw.ir.locals[local].parameter.is_some()
            || raw.ir.locals[local].is_this
            || conflicts[local]
        {
            return;
        }
        // javac starts a new variable's range immediately after its initial
        // store. At a reused slot, that exact start owns the new definition even
        // if the preceding variable's range still covers the store opcode.
        let starts_after_store = after_store.is_some_and(|next| {
            by_slot
                .get(&slot)
                .into_iter()
                .flatten()
                .any(|&index| u32::from(variables[index].start_pc) == next)
        });
        let mut candidate = None;
        for &index in by_slot.get(&slot).into_iter().flatten() {
            let variable = &variables[index];
            let start = u32::from(variable.start_pc);
            let end = start + u32::from(variable.length);
            if if starts_after_store {
                after_store != Some(start)
            } else {
                !(start <= pc && pc < end)
            } {
                continue;
            }
            let actual = &raw.ir.locals[local].ty;
            if !declared_types[index].as_ref().is_some_and(|declared| {
                *declared != JvmType::Void
                    && ((declared.is_int_like() && actual.is_int_like())
                        || (declared.is_reference() && actual.is_reference())
                        || declared == actual)
            }) {
                conflicts[local] = true;
                return;
            }
            // A slot's original parameter name does not name a later SSA value
            // stored into that slot, even when javac's entry range covers it.
            if start == 0 && parameter_slots.contains(&slot) {
                conflicts[local] = true;
                return;
            }
            match candidate {
                Some(previous) if !same_identity(&variables[previous], variable) => {
                    conflicts[local] = true;
                    return;
                }
                _ => candidate = Some(index),
            }
        }
        if let Some(index) = candidate {
            if !presentation
                && candidates[local]
                    .keys()
                    .any(|&previous| !same_identity(&variables[previous], &variables[index]))
            {
                conflicts[local] = true;
                return;
            }
            *candidates[local].entry(index).or_default() += 1;
        }
    };
    let mut exact_access: IdVec<LocalId, _> = vec![false; raw.ir.locals.len()].into();
    for access in &raw.local_accesses {
        let Some(local) = raw.ir.values[access.value].local else {
            continue;
        };
        exact_access[local] = true;
        let (pc, next_pc) = original_pcs
            .and_then(|pcs| pcs.get(access.pc as usize))
            .copied()
            .unwrap_or((access.pc, access.next_pc));
        observe(local, access.slot, pc, access.store.then_some(next_pc));
    }
    for value in &raw.ir.values {
        let binding = &value.local;
        if let ValueKind::Phi {
            slot: Some(slot), ..
        } = value.kind
            && let Some(local) = *binding
            && !exact_access[local]
        {
            let pc = original_pcs
                .and_then(|pcs| pcs.get(value.pc as usize))
                .map_or(value.pc, |&(pc, _)| pc);
            observe(local, slot, pc, None);
        }
    }
    for (local, candidates) in candidates.into_iter_enumerated() {
        if !conflicts[local] {
            // Prefer the lifetime with the most exact accesses. Stable table
            // order breaks ties without manufacturing a source-variable role.
            let candidate = candidates
                .into_iter()
                .max_by_key(|&(index, count)| (count, std::cmp::Reverse(index)))
                .map(|(index, _)| index);
            raw.ir.locals[local].name_hint = candidate.map(|index| variables[index].clone());
        }
    }
}

fn same_identity(left: &LocalVariable, right: &LocalVariable) -> bool {
    left.index == right.index
        && left.start_pc == right.start_pc
        && left.length == right.length
        && left.name == right.name
        && left.descriptor == right.descriptor
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::test_ids::*;
    use crate::ir::lift::LocalAccess;
    use crate::ir::{Literal, Local, MethodIr, Value};

    fn raw(accesses: Vec<LocalAccess>) -> RawIr {
        let mut raw = RawIr::new({
            let mut ir = MethodIr {
                entry: B0,
                blocks: vec![].into(),
                values: vec![Value {
                    local: None,
                    kind: ValueKind::Literal(Literal::Int(7)),
                    ty: JvmType::Int,
                    pc: 2,
                    block: B0,
                }]
                .into(),
                locals: vec![Local {
                    name_hint: None,
                    name: "v0".into(),
                    ty: JvmType::Int,
                    parameter: None,
                    is_this: false,
                    needs_default: false,
                }]
                .into(),
                parameters: vec![],
                return_type: JvmType::Void,
                initialization_sites: vec![],
            };
            for (entity, metadata) in ir.values.iter_mut().zip(vec![Some(0)]) {
                entity.local = metadata.map(LocalId::from_index);
            }
            ir
        });
        raw.local_accesses = accesses;
        raw
    }

    fn variable(name: &str, slot: u16, start: u16, length: u16) -> LocalVariable {
        LocalVariable {
            name: name.into(),
            index: slot,
            start_pc: start,
            length,
            descriptor: "I".into(),
        }
    }

    #[test]
    fn initial_store_belongs_to_the_range_starting_after_it() {
        let mut raw = raw(vec![
            LocalAccess {
                value: V0,
                replaced: None,
                slot: 1,
                pc: 3,
                next_pc: 4,
                store: true,
            },
            LocalAccess {
                value: V0,
                replaced: None,
                slot: 1,
                pc: 5,
                next_pc: 6,
                store: false,
            },
        ]);
        assign(
            &mut raw,
            &[variable("old", 1, 1, 3), variable("new", 1, 4, 8)],
            None,
        );
        let hint = raw.ir.locals[L0].name_hint.as_ref().unwrap();
        assert_eq!(
            (hint.name.as_str(), hint.start_pc, hint.index),
            ("new", 4, 1)
        );
    }

    #[test]
    fn aliases_with_distinct_debug_lifetimes_have_no_single_identity() {
        let mut raw = raw(vec![
            LocalAccess {
                value: V0,
                replaced: None,
                slot: 1,
                pc: 3,
                next_pc: 4,
                store: true,
            },
            LocalAccess {
                value: V0,
                replaced: None,
                slot: 2,
                pc: 9,
                next_pc: 10,
                store: true,
            },
        ]);
        assign(
            &mut raw,
            &[variable("first", 1, 4, 4), variable("second", 2, 10, 4)],
            None,
        );
        assert!(raw.ir.locals[L0].name_hint.is_none());
        assign_final(
            &mut raw,
            &[variable("first", 1, 4, 4), variable("second", 2, 10, 4)],
            None,
        );
        assert_eq!(raw.ir.locals[L0].name_hint.as_ref().unwrap().name, "first");
    }

    #[test]
    fn finalized_storage_reuses_the_most_observed_proven_lifetime() {
        let mut raw = raw(vec![
            LocalAccess {
                value: V0,
                replaced: None,
                slot: 1,
                pc: 3,
                next_pc: 4,
                store: true,
            },
            LocalAccess {
                value: V0,
                replaced: None,
                slot: 2,
                pc: 9,
                next_pc: 10,
                store: true,
            },
            LocalAccess {
                value: V0,
                replaced: None,
                slot: 2,
                pc: 11,
                next_pc: 12,
                store: false,
            },
        ]);
        assign_final(
            &mut raw,
            &[variable("firstLine", 1, 4, 4), variable("line", 2, 10, 4)],
            None,
        );
        assert_eq!(raw.ir.locals[L0].name_hint.as_ref().unwrap().name, "line");
    }

    #[test]
    fn conflicting_metadata_for_one_exact_access_never_becomes_a_display_hint() {
        let mut raw = raw(vec![LocalAccess {
            value: V0,
            replaced: None,
            slot: 1,
            pc: 4,
            next_pc: 5,
            store: false,
        }]);
        assign_final(
            &mut raw,
            &[variable("first", 1, 4, 4), variable("second", 1, 4, 4)],
            None,
        );
        assert!(raw.ir.locals[L0].name_hint.is_none());
    }

    #[test]
    fn a_phi_position_cannot_override_exact_access_identity_at_a_lifetime_boundary() {
        let mut raw = raw(vec![LocalAccess {
            value: V0,
            replaced: None,
            slot: 1,
            pc: 4,
            next_pc: 5,
            store: false,
        }]);
        raw.ir.values.push(Value {
            local: None,
            kind: ValueKind::Phi {
                inputs: vec![],
                slot: Some(1),
            },
            ty: JvmType::Int,
            pc: 3,
            block: B0,
        });
        raw.ir.values.iter_mut().last().unwrap().local = Some(L0);
        assign_final(
            &mut raw,
            &[variable("old", 1, 1, 3), variable("current", 1, 4, 4)],
            None,
        );
        assert_eq!(
            raw.ir.locals[L0].name_hint.as_ref().unwrap().name,
            "current"
        );
    }

    #[test]
    fn invalid_or_incompatible_debug_descriptors_are_not_identity_evidence() {
        for descriptor in ["Q", "V", "J", "Ljava/lang/Object;"] {
            let mut raw = raw(vec![LocalAccess {
                value: V0,
                replaced: None,
                slot: 1,
                pc: 3,
                next_pc: 4,
                store: true,
            }]);
            let mut hint = variable("value", 1, 4, 8);
            hint.descriptor = descriptor.into();
            assign(&mut raw, &[hint], None);
            assert!(raw.ir.locals[L0].name_hint.is_none(), "{descriptor}");
        }
    }
}
