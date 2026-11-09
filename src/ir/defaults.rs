//! Own fresh literal defaults before coalescing without splitting carried locals.
//! Separate search results from induction while preserving original storage.
//! Ordered liveness protects every intervening write and exception observation.
use super::{Literal, MethodIr, Statement, Terminator, ValueId, ValueKind, scheduled_liveness};
use crate::cfg::Dominators;
use crate::index::{BlockId, IdVec, LocalId};
use std::collections::{BTreeMap, BTreeSet};

const MAX_ANALYSIS_VISITS: usize = 4_000_000;

/// An original initializer transported to later same-slot phi edges already
/// owns its storage. Preserve that family's literal writes at their definitions,
/// including updates before cleanup. Same-block edge copies own ordinary seeds.
pub(super) fn definitions(
    raw: &super::lift::RawIr,
    used: &IdVec<ValueId, bool>,
) -> BTreeMap<ValueId, crate::classfile::JvmType> {
    let ir = &raw.ir;
    let stores: BTreeSet<_> = raw
        .local_accesses
        .iter()
        .filter(|access| {
            access.store
                && matches!(
                    ir.values[access.value].kind,
                    ValueKind::Literal(
                        Literal::Null
                            | Literal::Int(_)
                            | Literal::Long(_)
                            | Literal::Float(_)
                            | Literal::Double(_)
                            | Literal::String(_)
                    )
                )
        })
        .map(|access| (access.value, access.slot))
        .collect();
    let families: Vec<_> = ir
        .values
        .iter_enumerated()
        .filter_map(|(id, value)| match &value.kind {
            ValueKind::Phi {
                inputs,
                slot: Some(slot),
            } if used[id] => Some((id, inputs, *slot)),
            _ => None,
        })
        .collect();
    if stores.is_empty() || families.is_empty() {
        return BTreeMap::new();
    }
    let (_, dom) = dominance(ir);
    let mut pending = Vec::new();
    for (id, inputs, slot) in families {
        let Some(common) = dom.common(inputs.iter().map(|input| input.block)) else {
            continue;
        };
        let first_throw = inputs
            .iter()
            .filter(|input| input.block == common)
            .filter_map(|input| input.throw_pc)
            .min()
            .unwrap_or(u32::MAX);
        if inputs.iter().any(|input| {
            let value = &ir.values[input.value];
            stores.contains(&(input.value, slot))
                && input.block != value.block
                && dom.dominates(value.block, common)
                && (value.block != common || value.pc <= first_throw)
        }) {
            pending.push((id, slot));
        }
    }
    let mut seen = BTreeSet::new();
    let mut captures: BTreeMap<_, Option<crate::classfile::JvmType>> = BTreeMap::new();
    while let Some((id, slot)) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        let ValueKind::Phi { inputs, .. } = &ir.values[id].kind else {
            unreachable!()
        };
        for input in inputs {
            let value = &ir.values[input.value];
            if let ValueKind::Phi {
                slot: Some(source), ..
            } = value.kind
            {
                if source == slot {
                    pending.push((input.value, slot));
                }
            } else if stores.contains(&(input.value, slot)) && value.local.is_none() {
                let ty = &ir.values[id].ty;
                if value.ty != *ty
                    && !(matches!(value.kind, ValueKind::Literal(Literal::Null))
                        && ty.is_reference())
                    && !(matches!(value.kind, ValueKind::Literal(Literal::Int(0 | 1)))
                        && *ty == crate::classfile::JvmType::Boolean)
                {
                    continue;
                }
                captures
                    .entry(input.value)
                    .and_modify(|prior| {
                        if prior.as_ref() != Some(ty) {
                            *prior = None;
                        }
                    })
                    .or_insert_with(|| Some(ty.clone()));
            }
        }
    }
    captures
        .into_iter()
        .filter_map(|(value, ty)| ty.map(|ty| (value, ty)))
        .collect()
}

fn dominance(ir: &MethodIr) -> (IdVec<BlockId, Vec<BlockId>>, Dominators) {
    let predecessors = scheduled_liveness::predecessors(ir);
    let mut successors: IdVec<BlockId, Vec<BlockId>> = vec![Vec::new(); ir.blocks.len()].into();
    for (target, sources) in predecessors.iter_enumerated() {
        for &source in sources {
            successors[source].push(target);
        }
    }
    let dom = Dominators::new(&successors, &predecessors, ir.entry);
    (predecessors, dom)
}

pub(super) fn hoist(raw: &mut super::lift::RawIr) {
    let ir = &raw.ir;
    let effects = scheduled_liveness::effects(ir);
    let mut writes: IdVec<LocalId, Vec<BlockId>> = vec![Vec::new(); ir.locals.len()].into();
    let mut copies: IdVec<LocalId, Vec<(LocalId, ValueId)>> =
        vec![Vec::new(); ir.locals.len()].into();
    let mut candidates = Vec::new();
    for (block, body) in ir.blocks.iter_enumerated() {
        for effect in effects[block]
            .statements
            .iter()
            .chain([&effects[block].terminal])
        {
            effect.assignments(&mut |local, value| {
                writes[local].push(block);
                if let Some(source) = value.local(ir) {
                    copies[local].push((source, value.value()));
                }
            });
        }
        let Terminator::Goto(tail) = body.terminator else {
            continue;
        };
        for (index, statement) in body.statements.iter().enumerate() {
            if let Statement::Assign { local, value } = statement
                && !ir.locals[*local].is_this
                && ir.locals[*local].parameter.is_none()
                && super::available_copies::is_literal_store(ir, statement)
            {
                candidates.push((block, index, tail, *local, value.value()));
            }
        }
    }
    if candidates.is_empty() {
        return;
    }
    let (predecessors, dom) = dominance(ir);
    let slots = super::local_identity::SourceSlots::new(ir);
    for sites in &mut writes {
        sites.retain(|&block| dom.idom[block].is_some());
    }
    let caught: BTreeSet<_> = ir
        .blocks
        .iter()
        .filter_map(|block| block.caught_exception)
        .filter_map(|value| ir.values[value].local)
        .collect();
    let locals: BTreeSet<_> = candidates.iter().map(|candidate| candidate.3).collect();
    let mut position: IdVec<LocalId, _> = vec![None; ir.locals.len()].into();
    for (index, &local) in locals.iter().enumerate() {
        position[local] = Some(index);
    }
    let mut budget = MAX_ANALYSIS_VISITS;
    let Some(live) = scheduled_liveness::analyze(
        ir,
        &effects,
        &predecessors,
        &position,
        locals.len(),
        &mut budget,
    ) else {
        return;
    };
    let owners: IdVec<LocalId, _> = writes
        .iter()
        .map(|sites| dom.common(sites.iter().copied()))
        .collect();
    candidates.sort_by_key(|&(prefix, _, _, local, value)| {
        (
            local,
            owners[local] != Some(ir.values[value].block),
            ir.values[value].pc,
            prefix,
        )
    });
    let mut planned = Vec::new();
    let mut chosen = BTreeSet::new();
    for (prefix, index, tail, local, _) in candidates {
        if chosen.contains(&local)
            || caught.contains(&local)
            || predecessors[tail].len() < 2
            || !live.entries[tail].contains(position[local].unwrap())
            || ir.blocks.iter().any(|block| {
                crate::cfg::dispatch::effective(&block.handlers)
                    .any(|handler| handler.target == prefix)
            })
        {
            continue;
        }
        let Some(mut owner) = owners[local] else {
            continue;
        };
        // Acyclic branches can belong to a lazy source expression. Only a
        // statement control owner needs a separately initialized result.
        if !matches!(ir.blocks[owner].terminator, Terminator::Switch { .. })
            && !predecessors[owner]
                .iter()
                .any(|&source| dom.dominates(owner, source))
        {
            continue;
        }
        // A terminal value can stay on its exit and become a direct return.
        // Materializing it before a search would create storage with no owner
        // once the successful paths have returned their own values.
        if ir.blocks[tail].statements.is_empty()
            && matches!(ir.blocks[tail].terminator, Terminator::Return(Some(value))
                if ir.values[value].local == Some(local))
            && outside_loop(owner, prefix, &predecessors, &dom)
        {
            continue;
        }
        // A search result belongs outside the loop that produces it. A default
        // consumed inside that same loop must still execute on each iteration.
        while owner != ir.entry && outside_loop(owner, prefix, &predecessors, &dom) {
            owner = dom.idom[owner].unwrap();
        }
        // Carried input keeps its slot or feeds an unnamed stack result.
        // Induction starts inside the loop, after a fresh result's outer owner.
        if owner == prefix
            || !dom.dominates(owner, prefix)
            || retains_storage(local, &copies, &mut budget, |value| {
                dom.dominates(ir.values[value].block, owner)
                    && slots.slot(local).is_none_or(|slot| {
                        super::local_identity::incoming_local(raw, owner, value, slot)
                    })
            })
        {
            continue;
        }
        let Some(before) = live.before_terminator(ir, &effects, owner, &position, &mut budget)
        else {
            break;
        };
        if before.contains(position[local].unwrap())
            || writes_local(&effects[owner].terminal, local)
            || effects[prefix].statements[..index]
                .iter()
                .any(|effect| writes_local(effect, local))
            || !clear_path(
                owner,
                prefix,
                local,
                &effects,
                &predecessors,
                &dom,
                &mut budget,
            )
        {
            continue;
        }
        chosen.insert(local);
        planned.push((owner, prefix, index));
    }
    // Distinct destination defaults commute. Preserve each original statement
    // identity so later copy elimination and initialization checks see its use.
    planned.sort_by_key(|&(_, prefix, index)| (prefix, std::cmp::Reverse(index)));
    for (owner, prefix, index) in planned {
        let statement = raw.remove_statement(prefix, index);
        raw.extend_statements(owner, [statement]);
    }
}

fn retains_storage(
    local: LocalId,
    copies: &IdVec<LocalId, Vec<(LocalId, ValueId)>>,
    budget: &mut usize,
    prior: impl Fn(ValueId) -> bool,
) -> bool {
    let mut pending = copies[local].clone();
    let mut seen = BTreeSet::new();
    while let Some((source, origin)) = pending.pop() {
        if source == local || prior(origin) || *budget == 0 {
            return true;
        }
        *budget -= 1;
        if seen.insert(source) {
            pending.extend(copies[source].iter().copied());
        }
    }
    false
}

fn writes_local(effect: &super::local_effects::Effect, local: LocalId) -> bool {
    let mut writes = false;
    effect.writes(&mut |written| writes |= written == local);
    writes
}

fn clear_path(
    owner: BlockId,
    prefix: BlockId,
    local: LocalId,
    effects: &IdVec<BlockId, scheduled_liveness::BlockEffects>,
    predecessors: &IdVec<BlockId, Vec<BlockId>>,
    dom: &Dominators,
    budget: &mut usize,
) -> bool {
    let mut seen: IdVec<BlockId, _> = vec![false; predecessors.len()].into();
    seen[prefix] = true;
    let mut pending = predecessors[prefix].clone();
    while let Some(node) = pending.pop() {
        if node == owner || std::mem::replace(&mut seen[node], true) || dom.idom[node].is_none() {
            continue;
        }
        let Some(remaining) = budget.checked_sub(1) else {
            return false;
        };
        *budget = remaining;
        if effects[node]
            .statements
            .iter()
            .chain([&effects[node].terminal])
            .any(|effect| writes_local(effect, local))
        {
            return false;
        }
        pending.extend(predecessors[node].iter().copied());
    }
    true
}

fn outside_loop(
    header: BlockId,
    prefix: BlockId,
    predecessors: &IdVec<BlockId, Vec<BlockId>>,
    dom: &Dominators,
) -> bool {
    let mut pending: Vec<_> = predecessors[header]
        .iter()
        .copied()
        .filter(|&node| dom.dominates(header, node))
        .collect();
    if pending.is_empty() {
        return false;
    }
    let mut seen: IdVec<BlockId, _> = vec![false; predecessors.len()].into();
    seen[header] = true;
    while let Some(node) = pending.pop() {
        if node == prefix {
            return false;
        }
        if !std::mem::replace(&mut seen[node], true) && dom.dominates(header, node) {
            pending.extend(predecessors[node].iter().copied());
        }
    }
    true
}

#[cfg(test)]
#[path = "../../tests/unit/defaults.rs"]
mod tests;
