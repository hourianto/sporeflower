//! Typed out-of-SSA storage coalescing over scheduled physical local accesses.
//! A copy is removable only when its endpoints have no interfering writes on
//! normal or exceptional paths. Immutable values are never unified.
#[cfg(test)]
use super::ValueKind;
use super::lift::RawIr;
use super::local_effects::Effect;
use super::local_identity::SourceSlots;
use super::scheduled_liveness::{self, Bits, BlockEffects};
#[cfg(test)]
use super::scheduled_liveness::{Transfer, summarize};
use super::{LocalId, MethodIr, Statement};
use crate::hash::HashMap;
use crate::index::{BlockId, IdVec};
use std::cell::OnceCell;
use std::collections::BTreeMap;

mod nulls;

const MAX_COMPONENT_LOCALS: usize = 512;
const MAX_COMPONENT_VISITS: usize = 4_000_000;
const MAX_METHOD_VISITS: usize = 16_000_000;

#[derive(Clone, Copy)]
struct Copy {
    block: BlockId,
    statement: usize,
    destination: LocalId,
    source: LocalId,
}

pub(super) fn run(raw: &mut RawIr) -> usize {
    let ir = &mut raw.ir;
    nulls::refine(ir);
    let source_slots = SourceSlots::new(ir);
    let mut copies = Vec::new();
    let mut parent: IdVec<LocalId, _> = (ir.locals.ids()).collect();
    for (block, body) in ir.blocks.iter_enumerated() {
        for (statement, action) in body.statements.iter().enumerate() {
            let Statement::Assign {
                local: destination,
                value,
            } = *action
            else {
                continue;
            };
            let Some(source) = value.local(ir) else {
                continue;
            };
            if source == destination || ir.locals[source].ty != ir.locals[destination].ty {
                continue;
            }
            copies.push(Copy {
                block,
                statement,
                destination,
                source,
            });
            if !ir.locals[source].is_this
                && !ir.locals[destination].is_this
                && hints_compatible(ir, Some(source), Some(destination))
            {
                unite(&mut parent, source, destination);
            }
        }
    }
    if copies.is_empty() {
        return 0;
    }
    let mut components = BTreeMap::<LocalId, Vec<LocalId>>::new();
    for local in ir.locals.ids() {
        let component = compress(&mut parent, local);
        components.entry(component).or_default().push(local);
    }
    let mut candidates: Vec<_> = components
        .into_values()
        .filter(|locals| locals.len() > 1 && locals.len() <= MAX_COMPONENT_LOCALS)
        .collect();
    if candidates.is_empty() {
        return 0;
    }
    let mut weights = HashMap::<(LocalId, LocalId), usize>::default();
    let mut benefits = HashMap::<LocalId, usize>::default();
    for copy in &copies {
        *weights.entry((copy.destination, copy.source)).or_default() += 1;
        let component = root(&parent, copy.destination);
        if component == root(&parent, copy.source) {
            *benefits.entry(component).or_default() += 1;
        }
    }
    candidates.sort_by_key(|locals| {
        (
            std::cmp::Reverse(
                benefits
                    .get(&root(&parent, locals[0]))
                    .copied()
                    .unwrap_or(0),
            ),
            locals[0],
        )
    });
    let effects = scheduled_liveness::effects(ir);
    let predecessors = scheduled_liveness::predecessors(ir);
    let mut mapping: IdVec<LocalId, _> = (ir.locals.ids()).collect();
    let mut metadata = Vec::new();
    let content_facts = OnceCell::new();
    let mut method_budget = MAX_METHOD_VISITS;
    for locals in candidates {
        if method_budget == 0 {
            break;
        }
        let words = locals.len().div_ceil(64);
        let mut position: IdVec<LocalId, _> = vec![None; ir.locals.len()].into();
        for (index, &local) in locals.iter().enumerate() {
            position[local] = Some(index);
        }
        let mut budget = method_budget.min(MAX_COMPONENT_VISITS);
        let original_budget = budget;
        let Some(interference) = interference(
            ir,
            &effects,
            &predecessors,
            &position,
            &locals,
            &content_facts,
            &mut budget,
        ) else {
            method_budget -= original_budget - budget;
            continue;
        };
        method_budget -= original_budget - budget;
        let mut groups: Vec<_> = locals
            .iter()
            .enumerate()
            .map(|(index, &local)| Group {
                members: Bits::one(words, index),
                neighbors: interference[index].clone(),
                representative: local,
                hint: hint(ir, local).map(|_| local),
                ambiguous: hint(ir, local).is_none(),
                needs_default: ir.locals[local].needs_default,
            })
            .collect();
        let mut parents: Vec<_> = (0..locals.len()).collect();
        let mut edges: Vec<_> = weights
            .iter()
            .filter_map(|(&(destination, source), &weight)| {
                position[destination]
                    .zip(position[source])
                    .map(|(destination, source)| (weight, destination, source))
            })
            .collect();
        edges.sort_by_key(|&(weight, destination, source)| {
            (
                std::cmp::Reverse(source_slots.same(locals[destination], locals[source])),
                std::cmp::Reverse(weight),
                destination,
                source,
            )
        });
        for (_, destination, source) in edges {
            let left = root(&parents, destination);
            let right = root(&parents, source);
            let left_parameter = ir.locals[groups[left].representative].parameter.is_some();
            let right_parameter = ir.locals[groups[right].representative].parameter.is_some();
            if left == right
                // Each argument is a distinct incoming binding, including an
                // unused one. Its signature and initial contents must survive.
                || left_parameter && right_parameter
                || groups[left].neighbors.intersects(&groups[right].members)
                || !hints_compatible(ir, groups[left].hint, groups[right].hint)
            {
                continue;
            }
            let (keep, remove) = if left_parameter
                || !right_parameter && groups[left].representative < groups[right].representative
            {
                (left, right)
            } else {
                (right, left)
            };
            let other = groups[remove].clone();
            parents[remove] = keep;
            groups[keep].members.union(&other.members);
            groups[keep].neighbors.union(&other.neighbors);
            groups[keep].hint = groups[keep].hint.or(other.hint);
            groups[keep].ambiguous |= other.ambiguous;
            groups[keep].needs_default |= other.needs_default;
        }
        for (index, &local) in locals.iter().enumerate() {
            let representative = root(&parents, index);
            mapping[local] = groups[representative].representative;
            if representative == index {
                metadata.push((
                    groups[index].representative,
                    groups[index].needs_default,
                    (!groups[index].ambiguous)
                        .then_some(groups[index].hint)
                        .flatten()
                        .and_then(|local| hint(ir, local).cloned()),
                ));
            }
        }
    }
    // Copies whose endpoints share storage are redundant. Their explicit read
    // remains a read throughout renaming; producer definitions are unaffected.
    let mut removed: IdVec<BlockId, _> = vec![Vec::new(); ir.blocks.len()].into();
    for copy in copies {
        if mapping[copy.destination] == mapping[copy.source] {
            removed[copy.block].push(copy.statement);
        }
    }
    let count: usize = removed.iter().map(Vec::len).sum();
    if count == 0 {
        return 0;
    }
    for (block, positions) in removed.iter_enumerated() {
        raw.retain_statements(block, |index| positions.binary_search(&index).is_err());
    }
    let ir = &mut raw.ir;
    for block in &mut ir.blocks {
        for statement in &mut block.statements {
            if let Statement::Assign { local, .. } = statement {
                *local = mapping[*local];
            }
        }
    }
    for value in &mut ir.values {
        if let Some((local, _)) = value.kind.local_write_mut() {
            *local = mapping[*local];
        }
        if let Some(local) = &mut value.local {
            *local = mapping[*local];
        }
    }
    for (local, needs_default, hint) in metadata {
        ir.locals[local].needs_default = needs_default;
        ir.locals[local].name_hint = hint;
    }
    count
}

#[derive(Clone)]
struct Group {
    members: Bits,
    neighbors: Bits,
    representative: LocalId,
    hint: Option<LocalId>,
    ambiguous: bool,
    needs_default: bool,
}

fn hint(ir: &MethodIr, local: LocalId) -> Option<&crate::classfile::LocalVariable> {
    ir.locals
        .get(local)
        .and_then(|local| local.name_hint.as_ref())
}

fn hints_compatible(ir: &MethodIr, left: Option<LocalId>, right: Option<LocalId>) -> bool {
    match (
        left.and_then(|local| hint(ir, local)),
        right.and_then(|local| hint(ir, local)),
    ) {
        (Some(left), Some(right)) => {
            left.index == right.index
                && left.start_pc == right.start_pc
                && left.length == right.length
                && left.name == right.name
                && left.descriptor == right.descriptor
        }
        _ => true,
    }
}

fn root<I: std::marker::Copy + Eq>(
    parent: &impl std::ops::Index<I, Output = I>,
    mut local: I,
) -> I {
    while parent[local] != local {
        local = parent[local];
    }
    local
}

fn unite<I: std::marker::Copy + Ord>(
    parent: &mut impl std::ops::IndexMut<I, Output = I>,
    left: I,
    right: I,
) {
    let left = compress(parent, left);
    let right = compress(parent, right);
    parent[left.max(right)] = left.min(right);
}

fn compress<I: std::marker::Copy + Eq>(
    parent: &mut impl std::ops::IndexMut<I, Output = I>,
    mut local: I,
) -> I {
    let representative = root(parent, local);
    while parent[local] != local {
        let next = parent[local];
        parent[local] = representative;
        local = next;
    }
    representative
}

fn interference(
    ir: &MethodIr,
    effects: &IdVec<BlockId, BlockEffects>,
    predecessors: &IdVec<BlockId, Vec<BlockId>>,
    position: &IdVec<LocalId, Option<usize>>,
    locals: &[LocalId],
    content_facts: &OnceCell<super::AvailableCopies>,
    budget: &mut usize,
) -> Option<Vec<Bits>> {
    let count = locals.len();
    let words = count.div_ceil(64);
    let live = scheduled_liveness::analyze(ir, effects, predecessors, position, count, budget)?;
    // Parameters are defined by method entry. Other entry-live storage lacks
    // a modeled incoming definition, so its component stays untouched. The
    // merge step retains each parameter as its group's only incoming binding.
    let mut unknown_entry = false;
    live.entries[ir.entry].each(|index| {
        unknown_entry |= ir.locals[locals[index]].parameter.is_none();
    });
    if unknown_entry {
        return None;
    }
    let mut edges = vec![Bits::zero(words); count];
    for block in ir.blocks.ids() {
        let (mut before, exceptional) = live.outgoing(ir, block, position);
        for handler in crate::cfg::dispatch::effective(&ir.blocks[block].handlers) {
            if let Some(local) = ir.blocks[handler.target]
                .caught_exception
                .and_then(|value| ir.values[value].local)
                .and_then(|local| position[local])
            {
                add_edges(&mut edges, local, &live.entries[handler.target], None);
            }
        }
        reverse(
            &effects[block].terminal,
            &mut before,
            &exceptional,
            position,
            Some(&mut Conflicts {
                edges: &mut edges,
                same_contents: None,
            }),
            budget,
        )?;
        for (index, effect) in effects[block].statements.iter().enumerate().rev() {
            // Whole copies/literal stores have no expression-internal local
            // writes. Only a new prospective conflict asks for their incoming
            // content view; exception liveness and all other writes stay exact.
            let candidate = super::available_copies::has_assignment_contents(
                ir,
                &ir.blocks[block].statements[index],
            );
            let mut incoming = None;
            let mut same_contents = |peer| {
                incoming
                    .get_or_insert_with(|| {
                        content_facts
                            .get_or_init(|| super::AvailableCopies::analyze(ir))
                            .assignment_contents(ir, block, index)
                    })
                    .as_mut()
                    .is_some_and(|contents| contents.held_by(locals[peer]))
            };
            reverse(
                effect,
                &mut before,
                &exceptional,
                position,
                Some(&mut Conflicts {
                    edges: &mut edges,
                    same_contents: candidate
                        .then_some(&mut same_contents as &mut dyn FnMut(usize) -> bool),
                }),
                budget,
            )?;
        }
    }
    Some(edges)
}

/// One newly created snapshot has no lifetime beyond this block. Reuse the
/// coalescer's ordered interference proof before sharing its predecessor's
/// storage; exception exits observe the exact prethrow write order.
pub(super) fn confined_copy(
    ir: &MethodIr,
    source: LocalId,
    destination: LocalId,
    effects: &[Effect],
    exits: &[std::collections::BTreeSet<LocalId>; 2],
    budget: &mut usize,
) -> bool {
    if exits.iter().any(|live| live.contains(&destination))
        || !hints_compatible(ir, Some(source), Some(destination))
    {
        return false;
    }
    let mut position: IdVec<LocalId, Option<usize>> = vec![None; ir.locals.len()].into();
    position[source] = Some(0);
    position[destination] = Some(1);
    let mut live = Bits::zero(1);
    let mut exceptional = Bits::zero(1);
    if exits[0].contains(&source) {
        live.insert(0);
    }
    if exits[1].contains(&source) {
        exceptional.insert(0);
    }
    let mut edges = vec![Bits::zero(1); 2];
    for effect in effects.iter().rev() {
        if reverse(
            effect,
            &mut live,
            &exceptional,
            &position,
            Some(&mut Conflicts {
                edges: &mut edges,
                same_contents: None,
            }),
            budget,
        )
        .is_none()
        {
            return false;
        }
    }
    !edges[0].intersects(&Bits::one(1, 1))
}

fn add_edges(edges: &mut [Bits], written: usize, live: &Bits, copied: Option<usize>) {
    Conflicts {
        edges,
        same_contents: None,
    }
    .add(written, live, copied);
}

struct Conflicts<'a> {
    edges: &'a mut [Bits],
    same_contents: Option<&'a mut dyn FnMut(usize) -> bool>,
}

impl Conflicts<'_> {
    fn add(&mut self, written: usize, live: &Bits, copied: Option<usize>) {
        live.each(|local| {
            if local != written
                && Some(local) != copied
                && self.edges[written].0[local / 64] & (1 << (local % 64)) == 0
                && !self.same_contents.as_mut().is_some_and(|same| same(local))
            {
                self.edges[written].insert(local);
                self.edges[local].insert(written);
            }
        });
    }
}

fn reverse(
    effect: &Effect,
    live: &mut Bits,
    exceptional: &Bits,
    position: &IdVec<LocalId, Option<usize>>,
    mut edges: Option<&mut Conflicts<'_>>,
    budget: &mut usize,
) -> Option<()> {
    if *budget == 0 {
        return None;
    }
    *budget -= 1;
    match effect {
        Effect::Read(local) => {
            if let Some(local) = position[*local] {
                live.insert(local);
            }
        }
        Effect::Write { local, copied, .. } => {
            if let Some(local) = position[*local] {
                if let Some(edges) = edges.as_mut() {
                    edges.add(local, live, copied.and_then(|local| position[local]));
                }
                live.remove(local);
            }
        }
        Effect::Throw => live.union(exceptional),
        Effect::Sequence(effects) => {
            for effect in effects.iter().rev() {
                reverse(
                    effect,
                    live,
                    exceptional,
                    position,
                    edges.as_deref_mut(),
                    budget,
                )?;
            }
        }
        Effect::Choice(left, right) => {
            let mut alternative = live.clone();
            reverse(
                left,
                live,
                exceptional,
                position,
                edges.as_deref_mut(),
                budget,
            )?;
            reverse(
                right,
                &mut alternative,
                exceptional,
                position,
                edges,
                budget,
            )?;
            live.union(&alternative);
        }
        Effect::Empty => {}
    }
    Some(())
}

#[cfg(test)]
#[path = "../../tests/unit/coalesce.rs"]
mod tests;
