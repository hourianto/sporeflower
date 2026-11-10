//! Contract fact construction, requirements, and array dimension projections.
//! These operations do not read or mutate solver state.
use super::super::{Facts, flow::NodeId, ranges};
use crate::classfile::JvmType;
use crate::hash::HashSet;
use crate::mappings::{ConstantValue, Contract, DomainId, DomainKind, MappingPlan};

pub(super) fn contract(contract: &Contract, ty: &JvmType) -> Facts {
    let mut facts = Facts::default();
    if let Some(container) = &contract.container {
        facts.containers.insert(container.clone());
    }
    if matches!(ty, JvmType::Array(_)) && has_array(contract, ty) {
        let mut shape = contract.clone();
        shape.container = None;
        shape.derived = None;
        facts.arrays.insert(shape);
    } else {
        facts.domains.extend(contract.leaf);
        facts.class_name = contract.class_name;
        // Object aliases can carry an explicit boxed meaning, but an array
        // shape is never invented solely from an Object declaration.
    }
    facts
}

fn slot_domain(mappings: &MappingPlan, slots: Option<DomainId>, slot: i64) -> Option<DomainId> {
    mappings
        .domain(slots?)
        .values
        .iter()
        .find(|value| value.value == ConstantValue::Integral(slot))
        .and_then(|value| value.slot_domain)
}
pub(super) fn column(mappings: &MappingPlan, source: Facts, dimension: usize, slot: u32) -> Facts {
    if source.is_empty() {
        return source;
    }
    let mut result = Facts {
        unknown: source.unknown || source.arrays.is_empty(),
        ..Facts::default()
    };
    for shape in source.arrays {
        let binding = shape.arrays.get(&dimension);
        let slots = binding.and_then(|binding| {
            binding
                .records
                .as_ref()
                .map(|layout| layout.domain)
                .or(binding.slots)
        });
        if binding
            .and_then(|binding| binding.records.as_ref())
            .is_some_and(|layout| !layout.planes && slot >= layout.stride)
        {
            result.unknown = true;
            continue;
        }
        let leaf = slot_domain(mappings, slots, i64::from(slot));
        if let Some(domain) = leaf.or(shape.leaf) {
            result.domains.insert(domain);
        } else {
            result.unknown = true;
        }
        if leaf.is_none() {
            for (_, outer) in shape.arrays.range(..dimension) {
                for domain in outer
                    .slots
                    .into_iter()
                    .chain(outer.records.as_ref().map(|layout| layout.domain))
                {
                    for slot in &mappings.domain(domain).values {
                        result.domains.extend(slot.slot_domain);
                    }
                }
            }
        }
    }
    result
}
fn next_dimension(shape: &Contract) -> Contract {
    let mut next = shape.clone();
    next.arrays = next
        .arrays
        .into_iter()
        .filter(|&(dimension, _)| dimension > 0)
        .map(|(dimension, binding)| (dimension - 1, binding))
        .collect();
    next
}

pub(super) fn project(
    mappings: &MappingPlan,
    queries: &ranges::Queries<'_>,
    shape: &Contract,
    index: NodeId,
    element: &JvmType,
) -> Facts {
    let graph = queries.graph;
    let next = next_dimension(shape);
    let mut choices = HashSet::default();
    let mut unknown = false;
    if let Some(binding) = shape.arrays.get(&0) {
        let slot = binding
            .records
            .as_ref()
            .and_then(|layout| ranges::slot(queries, index, layout))
            .map(i64::from)
            .or_else(|| graph.integer(index));
        let slots = if binding.records.as_ref().is_some_and(|layout| {
            graph
                .integer(index)
                .is_some_and(|index| index < i64::from(layout.offset))
        }) {
            binding.slots
        } else {
            binding
                .records
                .as_ref()
                .map(|layout| layout.domain)
                .or(binding.slots)
        };
        if let Some(slot) = slot {
            choices.extend(slot_domain(mappings, slots, slot).or(shape.leaf));
        } else if let Some(slots) = slots {
            choices.extend(shape.leaf);
            for value in &mappings.domain(slots).values {
                choices.extend(value.slot_domain);
                if value.slot_domain.is_none() && shape.leaf.is_none() {
                    unknown = true;
                }
            }
            // Unlisted positions remain ordinary unless a leaf contract
            // supplies their default. A dynamic heterogeneous record stays
            // ambiguous even if only one named column has a domain.
            if shape.leaf.is_none() {
                unknown = true;
            }
        }
    }
    if choices.is_empty() {
        choices.extend(shape.leaf);
    }
    if matches!(element, JvmType::Array(_)) {
        let mut result = Facts {
            unknown,
            ..Facts::default()
        };
        if choices.is_empty() {
            result.arrays.insert(next);
        } else {
            for domain in choices {
                let mut next = next.clone();
                next.leaf = Some(domain);
                result.arrays.insert(next);
            }
        }
        result
    } else {
        Facts {
            unknown: unknown || choices.is_empty() && !shape.class_name,
            domains: choices,
            class_name: shape.class_name,
            ..Facts::default()
        }
    }
}

pub(super) fn project_constant(
    mappings: &MappingPlan,
    shape: &Contract,
    index: i64,
    element: &JvmType,
) -> Facts {
    let mut next = next_dimension(shape);
    if let Some(binding) = shape.arrays.get(&0) {
        let (slots, index) = if let Some(layout) = &binding.records {
            let adjusted = index - i64::from(layout.offset);
            if adjusted >= 0 {
                (
                    Some(layout.domain),
                    if layout.planes {
                        adjusted / i64::from(layout.stride)
                    } else {
                        adjusted % i64::from(layout.stride)
                    },
                )
            } else {
                (binding.slots, index)
            }
        } else {
            (binding.slots, index)
        };
        if let Some(domain) = slot_domain(mappings, slots, index) {
            next.leaf = Some(domain);
        }
    }
    if matches!(element, JvmType::Array(_)) {
        Facts {
            arrays: HashSet::from_iter([next]),
            ..Facts::default()
        }
    } else {
        Facts {
            domains: next.leaf.into_iter().collect(),
            class_name: next.class_name,
            ..Facts::default()
        }
    }
}

pub(super) fn fits(mappings: &MappingPlan, domain: DomainId, ty: &JvmType) -> bool {
    let domain = mappings.domain(domain);
    if domain.values.is_empty() {
        return false;
    }
    let flags = domain.kind == DomainKind::Flags;
    domain.values.iter().all(|value| {
        let ConstantValue::Integral(value) = value.value else {
            return false;
        };
        match ty {
            JvmType::Byte => {
                value >= i8::MIN.into()
                    && value
                        <= if flags {
                            u8::MAX.into()
                        } else {
                            i8::MAX.into()
                        }
            }
            JvmType::Short => {
                value >= i16::MIN.into()
                    && value
                        <= if flags {
                            u16::MAX.into()
                        } else {
                            i16::MAX.into()
                        }
            }
            JvmType::Char => {
                value >= if flags { i16::MIN.into() } else { 0 } && value <= u16::MAX.into()
            }
            JvmType::Int => {
                value >= i32::MIN.into()
                    && value
                        <= if flags {
                            u32::MAX.into()
                        } else {
                            i32::MAX.into()
                        }
            }
            JvmType::Long => true,
            _ => false,
        }
    })
}

pub(super) fn require(mut facts: Facts, requested: &Facts) -> Facts {
    if !facts.arrays.is_empty() && !requested.arrays.is_empty() {
        let mut shapes = HashSet::default();
        for source in &facts.arrays {
            for demand in &requested.arrays {
                if let Some(shape) = combine(source, demand) {
                    shapes.insert(shape);
                } else {
                    // Conflict is sticky evidence. Dropping both alternatives here
                    // would let the next demand resurrect one layout and make the
                    // result depend on traversal order (or oscillate forever).
                    shapes.insert(source.clone());
                    shapes.insert(demand.clone());
                    facts.unknown = true;
                }
            }
        }
        facts.arrays = shapes;
        let mut rest = requested.clone();
        rest.arrays.clear();
        facts.merge(&rest);
    } else {
        facts.merge(requested);
    }
    facts
}
fn combine(left: &Contract, right: &Contract) -> Option<Contract> {
    if left.leaf.is_some() && right.leaf.is_some() && left.leaf != right.leaf {
        return None;
    }
    if left.class_name != right.class_name && (left.leaf.is_some() || right.leaf.is_some()) {
        return None;
    }
    let mut result = left.clone();
    result.leaf = left.leaf.or(right.leaf);
    result.class_name |= right.class_name;
    for (&dimension, binding) in &right.arrays {
        let current = result.arrays.entry(dimension).or_default();
        if current.index.is_some() && binding.index.is_some() && current.index != binding.index
            || current.slots.is_some() && binding.slots.is_some() && current.slots != binding.slots
            || current.records.is_some()
                && binding.records.is_some()
                && current.records != binding.records
        {
            return None;
        }
        current.index = current.index.or(binding.index);
        current.slots = current.slots.or(binding.slots);
        current.records = current.records.clone().or_else(|| binding.records.clone());
        if current.index.is_some() && (current.slots.is_some() || current.records.is_some()) {
            return None;
        }
    }
    Some(result)
}
pub(super) fn has_array(contract: &Contract, ty: &JvmType) -> bool {
    !contract.arrays.is_empty()
        || matches!(ty, JvmType::Array(_)) && (contract.leaf.is_some() || contract.class_name)
}

pub(super) fn named_cast_preserves(
    queries: &ranges::Queries<'_>,
    value: NodeId,
    ty: &JvmType,
) -> bool {
    let graph = queries.graph;
    if !graph.nodes[value].ty.is_int_like() && graph.nodes[value].ty != JvmType::Long {
        return false;
    }
    let bounds = match ty {
        JvmType::Byte => (i64::from(i8::MIN), i64::from(i8::MAX)),
        JvmType::Short => (i64::from(i16::MIN), i64::from(i16::MAX)),
        JvmType::Char => (0, i64::from(u16::MAX)),
        JvmType::Int => (i64::from(i32::MIN), i64::from(i32::MAX)),
        _ => return false,
    };
    let range = queries.range(value);
    range.min >= bounds.0 && range.max <= bounds.1
}

pub(super) fn cast_preserves(source: &JvmType, target: &JvmType) -> bool {
    source == target
        || source.is_reference() && target.is_reference()
        || target == &JvmType::Long && source.is_int_like()
        || target == &JvmType::Int
            && matches!(source, JvmType::Byte | JvmType::Short | JvmType::Char)
}
