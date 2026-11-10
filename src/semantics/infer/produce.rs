//! Solve producer evidence before consumer requirements exist. Transfer reads
//! occurrence facts; only the fixed-point driver updates them.
use super::super::{
    Facts,
    flow::{NodeId, Op, Receiver},
    packed,
};
use super::{
    bindings::{Bindings, is_unboxing, is_wrapper, wrapper_type},
    evidence::{Dependent, Evidence},
    heap::{Heap, is_container},
    shape,
};
use crate::classfile::JvmType;
use crate::hash::HashSet;
use crate::ir::{BinaryOp, Member};
use crate::mappings::{ContainerContract, DerivedDomain, DomainKind, Target};

pub(super) struct Producer<'a, 'p> {
    pub evidence: Evidence<'a>,
    pub bindings: &'a Bindings<'a, 'p>,
    pub heap: &'a Heap,
}

impl Producer<'_, '_> {
    pub(super) fn compute(&self, node: NodeId) -> Facts {
        let graph = self.evidence.graph;
        let mappings = self.evidence.mappings;
        let data = &graph.nodes[node];
        match &data.op {
            Op::Literal(_) => Facts::default(),
            Op::Unknown => Facts::unknown(),
            Op::Parameter(index) => {
                let Some(contract) = mappings.contract(Target::Parameter {
                    method: self.bindings.member,
                    index: *index,
                }) else {
                    return Facts::unknown();
                };
                if let Some(DerivedDomain::Conditional(cases)) = &contract.derived
                    && let Some(selector) = cases
                        .first()
                        .and_then(|case| graph.parameters.get(&case.parameter))
                {
                    return Facts {
                        dependent: HashSet::from_iter([Dependent::Conditional {
                            selector: *selector,
                            cases: cases.clone(),
                        }]),
                        ..Facts::default()
                    };
                }
                let facts = shape::contract(contract, &data.ty);
                if facts.is_empty() {
                    Facts::unknown()
                } else {
                    facts
                }
            }
            Op::Read { sources, .. } => {
                if self.evidence.ranges.induction[node].is_some() {
                    return Facts::default();
                }
                if sources.is_empty() {
                    return Facts::unknown();
                }
                let mut result = Facts::default();
                for &source in sources {
                    result.merge(&self.evidence.at(source, node));
                }
                result
            }
            Op::Copy(value) => self.evidence.at(*value, node),
            Op::Select {
                left,
                right,
                comparison,
                yes,
                no,
            } => {
                if let Some((left, right)) = graph
                    .known_at(node, *left)
                    .zip(graph.known_at(node, *right))
                {
                    let choose_yes = comparison.test(Some(left.cmp(&right)));
                    self.evidence.at(if choose_yes { *yes } else { *no }, node)
                } else {
                    self.evidence
                        .at(*yes, node)
                        .joined(&self.evidence.at(*no, node))
                }
            }
            Op::Field(field) => {
                let mut facts = shape::contract(&self.bindings.field(field), &data.ty);
                if let Some(domain) = self
                    .bindings
                    .program
                    .resolve_field(&field.owner, &field.name, &field.descriptor)
                    .and_then(|field| mappings.real_value(field))
                {
                    facts.domains.insert(domain);
                }
                if facts.is_empty() {
                    Facts::unknown()
                } else {
                    facts
                }
            }
            Op::Cast { value, ty } => {
                if let Some(extracted) = self.extraction(node) {
                    return extracted;
                }
                let source = self.evidence.at(*value, node);
                if shape::cast_preserves(&graph.nodes[*value].ty, ty)
                    || shape::named_cast_preserves(self.evidence.ranges, *value, ty)
                    || source.is_empty()
                    || (!source.unknown
                        && !source.domains.is_empty()
                        && source
                            .domains
                            .iter()
                            .all(|&domain| shape::fits(mappings, domain, ty)))
                {
                    source
                } else {
                    Facts::unknown()
                }
            }
            Op::Binary { op, left, right } => {
                if let Some(extracted) = self.extraction(node) {
                    return extracted;
                }
                if matches!(op, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor) {
                    let source = self
                        .evidence
                        .at(*left, node)
                        .joined(&self.evidence.at(*right, node));
                    let flags: HashSet<_> = source
                        .domains
                        .iter()
                        .copied()
                        .filter(|&domain| mappings.domain(domain).kind == DomainKind::Flags)
                        .collect();
                    if flags.is_empty() {
                        return if source.is_empty() {
                            source
                        } else {
                            Facts::unknown()
                        };
                    }
                    Facts {
                        unknown: source.unknown
                            || flags.len() != source.domains.len()
                            || !source.dependent.is_empty(),
                        domains: flags,
                        ..Facts::default()
                    }
                } else {
                    Facts::unknown()
                }
            }
            Op::ArrayLoad { array, index } => {
                let facts = self.evidence.at(*array, node);
                let mut result = Facts {
                    unknown: facts.unknown,
                    ..Facts::default()
                };
                for shape in &facts.arrays {
                    result.merge(&shape::project(
                        mappings,
                        self.evidence.ranges,
                        shape,
                        *index,
                        &data.ty,
                    ));
                }
                if facts.arrays.is_empty() {
                    result.unknown = true;
                }
                result
            }
            Op::ArrayLength(_) => Facts::unknown(),
            Op::Call {
                method,
                receiver,
                args,
                offset,
            } => {
                if *receiver == Receiver::New && is_wrapper(&method.owner) && args.len() == 1 {
                    self.evidence.at(args[0], node)
                } else if *receiver == Receiver::New && is_container(&method.owner) {
                    if self.heap.escaped(node) {
                        Facts::unknown()
                    } else {
                        Facts::default()
                    }
                } else {
                    self.invocation(node, method, receiver.value(), args, *offset)
                }
            }
            Op::Array { elements } => {
                if self.heap.escaped(node) || self.heap.opaque(node) {
                    return Facts::unknown();
                }
                let mut result = Facts::default();
                for &element in elements {
                    result.merge(&self.lift_row(element, node));
                }
                for &(_, value) in self.heap.stores(node) {
                    result.merge(&self.lift_row(value, node));
                }
                if result.unknown {
                    Facts::unknown()
                } else {
                    result
                }
            }
        }
    }

    fn lift_row(&self, value: NodeId, outer: NodeId) -> Facts {
        let graph = self.evidence.graph;
        let facts = self.evidence.at(value, outer);
        if facts.is_empty() {
            return if matches!(graph.nodes[value].ty, JvmType::Array(_)) {
                Facts::unknown()
            } else {
                facts
            };
        }
        let rank = |ty: &JvmType| {
            let mut rank = 0;
            let mut ty = ty;
            while let JvmType::Array(inner) = ty {
                rank += 1;
                ty = inner;
            }
            rank
        };
        if rank(&graph.nodes[value].ty) + 1 != rank(&graph.nodes[outer].ty) {
            return Facts::unknown();
        }
        let mut result = Facts {
            unknown: facts.unknown
                || !facts.domains.is_empty()
                || !facts.dependent.is_empty()
                || !facts.containers.is_empty(),
            ..Facts::default()
        };
        for mut shape in facts.arrays {
            shape.arrays = shape
                .arrays
                .into_iter()
                .map(|(dimension, binding)| (dimension + 1, binding))
                .collect();
            result.arrays.insert(shape);
        }
        result
    }

    fn invocation(
        &self,
        node: NodeId,
        method: &Member,
        receiver: Option<NodeId>,
        args: &[NodeId],
        offset: u32,
    ) -> Facts {
        let graph = self.evidence.graph;
        let mappings = self.evidence.mappings;
        if let Some(domain) = mappings.call_domain(self.bindings.member, offset, None) {
            return Facts::scalar(domain);
        }
        let contract = self.bindings.method(method, None);
        let fixed = shape::contract(&contract, &graph.nodes[node].ty);
        if !fixed.is_empty() {
            return fixed;
        }
        match &contract.derived {
            Some(DerivedDomain::Parameter(index)) => {
                return args
                    .get(*index)
                    .map_or_else(Facts::unknown, |&argument| self.evidence.at(argument, node));
            }
            Some(DerivedDomain::Slot {
                parameter,
                dimension,
                slot,
            }) => {
                return args.get(*parameter).map_or_else(Facts::unknown, |&table| {
                    shape::column(mappings, self.evidence.at(table, node), *dimension, *slot)
                });
            }
            Some(DerivedDomain::Conditional(cases)) => {
                if let Some(&selector) = cases.first().and_then(|case| args.get(case.parameter)) {
                    return self.evidence.resolve(
                        node,
                        Facts {
                            dependent: HashSet::from_iter([Dependent::Conditional {
                                selector,
                                cases: cases.clone(),
                            }]),
                            ..Facts::default()
                        },
                        false,
                    );
                }
            }
            _ => {}
        }
        if is_wrapper(&method.owner) {
            if method.name == "valueOf" && args.len() == 1 {
                return self.evidence.at(args[0], node);
            }
            if is_unboxing(method)
                && let Some(receiver) = receiver
            {
                let source = self.evidence.at(receiver, node);
                if wrapper_type(&method.owner).as_ref() == Some(&graph.nodes[node].ty)
                    || source
                        .domains
                        .iter()
                        .all(|&domain| shape::fits(mappings, domain, &graph.nodes[node].ty))
                {
                    return source;
                }
            }
        }
        if let Some(receiver) = receiver
            && let Some(contents) = self.container_result(node, method, receiver)
        {
            return contents;
        }
        Facts::unknown()
    }

    fn container_result(&self, node: NodeId, method: &Member, receiver: NodeId) -> Option<Facts> {
        if !matches!(
            method.owner.as_str(),
            "java/util/Vector" | "java/util/Hashtable" | "java/util/Enumeration"
        ) {
            return None;
        }
        let source = self.evidence.at(receiver, node);
        let enumeration = matches!(method.name.as_str(), "elements" | "keys");
        let read = match method.owner.as_str() {
            "java/util/Vector" => matches!(
                method.name.as_str(),
                "elementAt" | "firstElement" | "lastElement"
            ),
            "java/util/Hashtable" => matches!(method.name.as_str(), "get" | "put" | "remove"),
            _ => method.name == "nextElement",
        };
        if !enumeration && !read {
            return None;
        }
        let mut result = Facts {
            unknown: source.unknown || source.containers.is_empty(),
            ..Facts::default()
        };
        for container in source.containers {
            let domain = if method.owner == "java/util/Hashtable" {
                if method.name == "keys" {
                    container.keys
                } else {
                    container.values
                }
            } else {
                container.elements
            };
            if let Some(domain) = domain {
                if enumeration {
                    result.containers.insert(ContainerContract {
                        elements: Some(domain),
                        ..ContainerContract::default()
                    });
                } else {
                    result.domains.insert(domain);
                }
            } else {
                result.unknown = true;
            }
        }
        Some(result)
    }

    fn extraction(&self, node: NodeId) -> Option<Facts> {
        let graph = self.evidence.graph;
        let mappings = self.evidence.mappings;
        let extraction = packed::extraction(graph, node)?;
        let source = self.evidence.layout_source(extraction.source, node);
        if !source
            .domains
            .iter()
            .any(|&domain| !mappings.domain(domain).bit_fields.is_empty())
        {
            return None;
        }
        let mut result = Facts {
            unknown: source.unknown,
            ..Facts::default()
        };
        let mut matched = false;
        for domain in source.domains {
            let candidates: Vec<_> = mappings
                .domain(domain)
                .bit_fields
                .iter()
                .filter(|field| {
                    field.shift == extraction.shift
                        && field.bits == extraction.bits
                        && field.signed == extraction.signed
                })
                .map(|field| (field.selector_mask, field.selector_value, field.domain))
                .collect();
            if candidates.is_empty() {
                result.unknown = true;
            } else {
                matched = true;
                result.dependent.insert(Dependent::Packed {
                    source: extraction.source,
                    cases: candidates,
                });
            }
        }
        matched.then(|| self.evidence.resolve(node, result, false))
    }
}
