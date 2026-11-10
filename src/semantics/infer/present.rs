//! Collapse frozen occurrence facts into the public presentation queries.
//! This stage owns ambiguity checks and names, but cannot feed facts back into
//! inference or mutate its producer layout snapshot.
use super::super::{
    Analysis, Facts, OffsetConstant, PackedConstant,
    flow::{NodeId, Op, Sink},
    packed,
};
use super::{
    evidence::Evidence,
    heap::Heap,
    shape::{cast_preserves, named_cast_preserves},
};
use crate::classfile::JvmType;
use crate::hash::{HashMap, HashSet};
use crate::ir::{Literal, LocalId, MethodIr, ValueId};
use crate::mappings::{DomainId, DomainKind};
use std::collections::BTreeMap;

pub(super) struct Solved<'a> {
    pub evidence: Evidence<'a>,
    pub ir: &'a MethodIr,
    pub heap: &'a Heap,
    pub required: &'a [Facts],
    pub packing_layouts: &'a [HashSet<DomainId>],
    pub ordinary: HashSet<NodeId>,
    pub offsets: &'a HashMap<NodeId, HashSet<Option<OffsetConstant>>>,
}

pub(super) fn empty() -> Analysis {
    Analysis {
        facts: HashMap::default(),
        sites: HashMap::default(),
        literals: crate::index::IdVec::new(),
        switches: BTreeMap::new(),
        packed: HashMap::default(),
        offsets: HashMap::default(),
        local_names: HashMap::default(),
        class_names: HashSet::default(),
        shared_class_names: HashSet::default(),
        class_name_groups: Vec::new(),
        diagnostics: Vec::new(),
    }
}

impl Solved<'_> {
    pub(super) fn finish(
        mut self,
        program: &crate::program::Program<'_>,
        member: crate::program::MemberId,
    ) -> Analysis {
        let mut result = empty();
        result.literals = vec![Facts::default(); self.ir.values.len()].into();
        // Ordinary string uses are a separate negative fact. A token-domain
        // consumer does not authorize relocating text shared with reflection.
        let mut pending: Vec<_> = self.ordinary.iter().copied().collect();
        while let Some(node) = pending.pop() {
            let children = match &self.evidence.graph.nodes[node].op {
                Op::Read { sources, .. } => sources.iter().copied().collect(),
                Op::Copy(value) | Op::Cast { value, .. } => vec![*value],
                Op::Select { yes, no, .. } => vec![*yes, *no],
                Op::Array { elements } => elements
                    .iter()
                    .copied()
                    .chain(self.heap.stores(node).iter().map(|store| store.1))
                    .collect(),
                _ => Vec::new(),
            };
            for child in children {
                if self.ordinary.insert(child) {
                    pending.push(child);
                }
            }
        }
        let mut class_names: HashMap<ValueId, (bool, bool)> = HashMap::default();
        for (node, data) in self.evidence.graph.nodes.iter().enumerate() {
            if !data.reachable {
                continue;
            }
            let Some(value) = data.value else {
                continue;
            };
            let facts = self.evidence.final_at(node, node);
            result
                .facts
                .entry((value, data.root))
                .or_default()
                .merge(&facts);
            result
                .sites
                .entry((value, data.root, data.site))
                .or_default()
                .merge(&facts);
            if matches!(data.op, Op::Literal(_)) {
                let requested = self
                    .evidence
                    .resolve(node, self.required[node].clone(), true);
                result.literals[value].merge(&requested);
                if matches!(data.op, Op::Literal(Literal::String(_))) {
                    let flags = class_names.entry(value).or_default();
                    flags.0 |= requested.class_name;
                    flags.1 |= self.ordinary.contains(&node) || !requested.class_name;
                }
            }
        }
        let mut shared = HashSet::default();
        for (value, (requested, ordinary)) in class_names {
            if requested && !ordinary {
                result.class_names.insert(value);
            } else if requested {
                shared.insert(value);
                result.diagnostics.push(format!("Class-name literal at bytecode {} is also used as ordinary text; its original spelling is preserved", self.ir.values[value].pc));
            }
        }
        super::super::class_names::plan(
            self.evidence.graph,
            self.ir,
            program,
            self.evidence.mappings,
            member,
            |node| {
                (
                    self.heap.is_unpublished(node) && !self.heap.opaque(node),
                    self.heap.stores(node),
                )
            },
            |array, value| {
                self.evidence
                    .final_at(array, value)
                    .arrays
                    .iter()
                    .any(|shape| shape.class_name)
            },
            shared,
            &mut result,
        );
        for sink in &self.evidence.graph.sinks {
            if let Sink::Switch { block, value } = sink {
                result
                    .switches
                    .entry(*block)
                    .or_default()
                    .merge(&self.evidence.final_at(*value, *value));
            }
        }
        self.present_packed(&mut result);
        let mut offsets: HashMap<ValueId, HashSet<Option<OffsetConstant>>> = HashMap::default();
        for (node, candidates) in self.offsets {
            if let Some(value) = self.evidence.graph.nodes[*node].value {
                offsets.entry(value).or_default().extend(candidates);
            }
        }
        for (value, candidates) in offsets {
            if candidates.len() == 1
                && result.literals[value].domains.is_empty()
                && let Some(Some(candidate)) = candidates.iter().next()
            {
                result.offsets.insert(value, *candidate);
            }
        }
        result
    }

    fn present_packed(&self, result: &mut Analysis) {
        let mut constants: HashMap<ValueId, HashSet<PackedConstant>> = HashMap::default();
        let mut names: HashMap<NodeId, (String, String)> = HashMap::default();
        for node in 0..self.evidence.graph.nodes.len() {
            self.present_clear_mask(node, &mut constants);
            if let Some(extraction) = packed::extraction(self.evidence.graph, node) {
                let source = self.evidence.layout_source(extraction.source, node);
                let mut candidates = Vec::new();
                let mut invalid = source.unknown || source.domains.is_empty();
                for domain in source.domains {
                    let mut found = false;
                    for (index, field) in self
                        .evidence
                        .mappings
                        .domain(domain)
                        .bit_fields
                        .iter()
                        .enumerate()
                    {
                        if field.shift != extraction.shift
                            || field.bits != extraction.bits
                            || field.signed != extraction.signed
                        {
                            continue;
                        }
                        if field.selector_mask != 0
                            && self
                                .evidence
                                .graph
                                .masked_at(node, extraction.source, field.selector_mask)
                                .is_some_and(|value| value != field.selector_value)
                        {
                            continue;
                        }
                        found = true;
                        if let Some(name) = &field.name {
                            candidates.push((domain, index, name.clone()));
                        } else {
                            invalid = true;
                        }
                    }
                    invalid |= !found;
                }
                if !invalid
                    && let Some((domain, field, name)) = candidates.first()
                    && candidates
                        .iter()
                        .all(|(other, _, other_name)| other == domain && other_name == name)
                {
                    names.insert(node, (format!("{}.{}", domain.0, name), name.clone()));
                    for &(operand, part) in &extraction.operands {
                        self.packed_candidate(
                            &mut constants,
                            operand,
                            PackedConstant {
                                domain: *domain,
                                field: *field,
                                part,
                            },
                        );
                    }
                }
            }
            for &domain in self.required[node]
                .domains
                .iter()
                .chain(&self.packing_layouts[node])
            {
                if self.evidence.mappings.domain(domain).kind != DomainKind::Packed {
                    continue;
                }
                for (field, entry) in self
                    .evidence
                    .mappings
                    .domain(domain)
                    .bit_fields
                    .iter()
                    .enumerate()
                {
                    if entry.name.is_none() {
                        continue;
                    }
                    if let Some(packing) = packed::packing(self.evidence.graph, node, entry) {
                        for (operand, part) in packing.operands {
                            self.packed_candidate(
                                &mut constants,
                                operand,
                                PackedConstant {
                                    domain,
                                    field,
                                    part,
                                },
                            );
                        }
                    }
                }
            }
            let layout = self.evidence.layout_source(node, node);
            if let Some(domain) = layout.domain() {
                let domain = self.evidence.mappings.domain(domain);
                if domain.kind == DomainKind::Packed
                    && domain.bit_fields.iter().any(|field| field.name.is_some())
                {
                    names.entry(node).or_insert_with(|| {
                        (
                            domain.owner.clone(),
                            format!("packed{}", domain.owner.rsplit('/').next().unwrap()),
                        )
                    });
                }
            }
        }
        for (value, candidates) in constants {
            if candidates.len() == 1 && result.literals[value].domains.is_empty() {
                result
                    .packed
                    .insert(value, *candidates.iter().next().unwrap());
            }
        }
        let mut locals: HashMap<LocalId, Option<(String, String)>> = HashMap::default();
        for (node, data) in self.evidence.graph.nodes.iter().enumerate() {
            let Op::Read { local, .. } = data.op else {
                continue;
            };
            if self.ir.locals[local].parameter.is_some()
                || self.ir.locals[local].is_this
                || self.ir.locals[local].name_hint.is_some()
            {
                continue;
            }
            let name = semantic_name(self.evidence, node, &names, &mut HashSet::default());
            match locals.entry(local) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(name);
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    if *entry.get() != name {
                        entry.insert(None);
                    }
                }
            }
        }
        for (local, name) in locals {
            if let Some((_, name)) = name {
                result.local_names.insert(local, name);
            }
        }
    }

    fn present_clear_mask(
        &self,
        node: NodeId,
        constants: &mut HashMap<ValueId, HashSet<PackedConstant>>,
    ) {
        let Some(clearing) = packed::clearing(self.evidence.graph, node) else {
            return;
        };
        let Some(domain) = self.evidence.layout_source(clearing.source, node).domain() else {
            return;
        };
        let mut candidates = self
            .evidence
            .mappings
            .domain(domain)
            .bit_fields
            .iter()
            .enumerate()
            .filter(|(_, field)| {
                field.shift == clearing.shift
                    && field.bits == clearing.bits
                    && (field.selector_mask == 0
                        || self
                            .evidence
                            .graph
                            .masked_at(node, clearing.source, field.selector_mask)
                            .is_none_or(|value| value == field.selector_value))
            });
        let Some((field, first)) = candidates.next() else {
            return;
        };
        if first.name.is_none() || candidates.any(|(_, other)| other.name != first.name) {
            return;
        }
        self.packed_candidate(
            constants,
            clearing.constant,
            PackedConstant {
                domain,
                field,
                part: super::super::PackedPart::ClearMask,
            },
        );
    }

    fn packed_candidate(
        &self,
        constants: &mut HashMap<ValueId, HashSet<PackedConstant>>,
        node: NodeId,
        mut candidate: PackedConstant,
    ) {
        let domain = self.evidence.mappings.domain(candidate.domain);
        let field = &domain.bit_fields[candidate.field];
        // Selector-specific value domains can share a physical field name.
        // They generate one constant; their candidate identities must agree too.
        if field.name.is_some() {
            candidate.field = domain
                .bit_fields
                .iter()
                .position(|other| {
                    other.name == field.name
                        && other.shift == field.shift
                        && other.bits == field.bits
                        && other.signed == field.signed
                })
                .unwrap();
        }
        let value = match candidate.part {
            super::super::PackedPart::Mask => packed::low_mask(field.bits) << field.shift,
            super::super::PackedPart::ClearMask => !(packed::low_mask(field.bits) << field.shift),
            super::super::PackedPart::ValueMask => packed::low_mask(field.bits),
            super::super::PackedPart::Shift => u64::from(field.shift),
        };
        let value = if self.evidence.graph.nodes[node].ty == JvmType::Long {
            value as i64
        } else {
            i64::from(value as i32)
        };
        if self.evidence.graph.integer(node) == Some(value)
            && let Some(id) = self.evidence.graph.nodes[node].value
        {
            constants.entry(id).or_default().insert(candidate);
        }
    }
}

fn semantic_name(
    evidence: Evidence<'_>,
    node: NodeId,
    names: &HashMap<NodeId, (String, String)>,
    visiting: &mut HashSet<NodeId>,
) -> Option<(String, String)> {
    let graph = evidence.graph;
    if let Some(name) = names.get(&node) {
        return Some(name.clone());
    }
    if !visiting.insert(node) {
        return None;
    }
    let sources = match &graph.nodes[node].op {
        Op::Read { sources, .. } => sources.iter().copied().collect(),
        Op::Copy(value) => vec![*value],
        Op::Cast { value, ty }
            if cast_preserves(&graph.nodes[*value].ty, ty)
                || named_cast_preserves(evidence.ranges, *value, ty) =>
        {
            vec![*value]
        }
        Op::Select { yes, no, .. } => vec![*yes, *no],
        _ => Vec::new(),
    };
    let mut result = None;
    for source in sources {
        let Some(name) = semantic_name(evidence, source, names, visiting) else {
            visiting.remove(&node);
            return None;
        };
        if result.as_ref().is_some_and(|old| *old != name) {
            visiting.remove(&node);
            return None;
        }
        result = Some(name);
    }
    visiting.remove(&node);
    result
}
