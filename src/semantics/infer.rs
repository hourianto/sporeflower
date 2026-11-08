//! Two-pass occurrence inference. The solver owns demands and scheduling;
//! producer transfer, heap analysis, and final presentation cannot mutate them.
mod bindings;
mod evidence;
mod heap;
mod present;
mod produce;
mod shape;

use super::{
    Analysis, Facts, OffsetConstant,
    flow::{Graph, NodeId, Op, Receiver, Sink},
    packed, ranges,
};
use crate::classfile::JvmType;
use crate::ir::{BinaryOp, CompareOp, Member, MethodIr};
use crate::mappings::{
    ContainerContract, Contract, DerivedDomain, DomainId, DomainKind, MappingPlan, Target,
};
use crate::program::{MemberId, Program};
use bindings::{Bindings, is_unboxing, is_wrapper, wrapper_type};
pub(super) use evidence::Dependent;
use evidence::Evidence;
use heap::{Heap, is_container};
use shape::require;
use std::collections::{HashMap, HashSet, VecDeque};

struct Solver<'a, 'p> {
    bindings: Bindings<'a, 'p>,
    graph: &'a Graph,
    ranges: ranges::Queries<'a>,
    // Current forward evidence with consumer requirements applied in pass two.
    facts: Vec<Facts>,
    // Frozen producer evidence cannot be redefined by consumer requirements.
    producer: Option<Vec<Facts>>,
    required: Vec<Facts>,
    readers: Vec<Vec<NodeId>>,
    uses: Vec<Vec<usize>>,
    worklist: Worklist,
    heap: Heap,
    ordinary: HashSet<NodeId>,
    // None records a producer alternative which does not support this layout.
    offsets: HashMap<NodeId, HashSet<Option<OffsetConstant>>>,
    packing_layouts: Vec<HashSet<DomainId>>,
    induction_peers: HashMap<super::flow::KeyRoot, Vec<NodeId>>,
}

struct Worklist {
    queue: VecDeque<NodeId>,
    queued: Vec<bool>,
}

impl Worklist {
    fn new(count: usize) -> Self {
        Self {
            queue: (0..count).collect(),
            queued: vec![true; count],
        }
    }
    fn enqueue(&mut self, node: NodeId) {
        if !self.queued[node] {
            self.queued[node] = true;
            self.queue.push_back(node);
        }
    }
    fn pop(&mut self) -> Option<NodeId> {
        let node = self.queue.pop_front()?;
        self.queued[node] = false;
        Some(node)
    }
}

pub(super) fn analyze(
    ir: &MethodIr,
    program: &Program<'_>,
    mappings: &MappingPlan,
    member: MemberId,
) -> Analysis {
    let bindings = Bindings {
        program,
        mappings,
        member,
    };
    if !bindings.has_evidence(ir) {
        return present::empty();
    }
    let graph = Graph::build(ir);
    let count = graph.nodes.len();
    let ranges = ranges::Queries::new(&graph);
    let mut induction_peers: HashMap<_, Vec<_>> = HashMap::new();
    for (node, counter) in ranges.induction.iter().enumerate() {
        if let Some(counter) = counter {
            induction_peers
                .entry(counter.root.clone())
                .or_default()
                .push(node);
        }
    }
    let heap = Heap::analyze(&graph, &bindings);
    let mut readers = vec![Vec::new(); count];
    for node in 0..count {
        for source in operands(&graph.nodes[node].op) {
            readers[source].push(node);
        }
        for &(index, value) in heap.stores(node) {
            readers[index].push(node);
            readers[value].push(node);
        }
    }
    let mut engine = Solver {
        bindings,
        graph: &graph,
        ranges,
        facts: vec![Facts::default(); count],
        producer: None,
        required: vec![Facts::default(); count],
        readers,
        uses: vec![Vec::new(); count],
        worklist: Worklist::new(count),
        heap,
        ordinary: HashSet::new(),
        offsets: HashMap::new(),
        packing_layouts: vec![HashSet::new(); count],
        induction_peers,
    };
    // Freeze the first fixed point before allowing consumers to contribute.
    // Both passes use the same transfer and scheduling engine; only the second
    // pass can create demands, presentation evidence or contextual layout facts.
    engine.solve();
    engine.producer = Some(engine.facts.clone());
    engine.worklist = Worklist::new(count);
    engine.index_uses();
    engine.solve();
    let ordinary = std::mem::take(&mut engine.ordinary);
    present::Solved {
        evidence: engine.evidence(),
        ir,
        heap: &engine.heap,
        required: &engine.required,
        packing_layouts: &engine.packing_layouts,
        ordinary,
        offsets: &engine.offsets,
    }
    .finish(program, member)
}

impl Solver<'_, '_> {
    fn producer(&self) -> &[Facts] {
        self.producer.as_deref().unwrap_or(&self.facts)
    }

    fn evidence(&self) -> Evidence<'_> {
        Evidence {
            graph: self.graph,
            ranges: &self.ranges,
            mappings: self.bindings.mappings,
            facts: &self.facts,
            layout: self.producer(),
        }
    }

    fn index_uses(&mut self) {
        for (index, sink) in self.graph.sinks.iter().enumerate() {
            for node in sink_operands(sink) {
                self.uses[node].push(index);
            }
        }
    }

    fn demand(&mut self, node: NodeId, facts: Facts) {
        if facts.is_empty() {
            return;
        }
        let peers = self.ranges.induction[node]
            .as_ref()
            .and_then(|counter| self.induction_peers.get(&counter.root))
            .cloned()
            .unwrap_or_else(|| vec![node]);
        for node in peers {
            let merged = require(self.required[node].clone(), &facts);
            if merged != self.required[node] {
                self.required[node] = merged;
                self.worklist.enqueue(node);
            }
        }
    }
    fn demand_source(&mut self, node: NodeId, mut facts: Facts) {
        // A later guard can select a captured packed/conditional value's
        // meaning. That choice belongs to the guarded read, not to the earlier
        // definition shared by every branch. Neutral fallback definitions still
        // receive the ordinary backward demand.
        if !self.producer()[node].dependent.is_empty() {
            facts.domains.clear();
        }
        self.demand(node, facts);
    }
    fn solve(&mut self) {
        let contextual = self.producer.is_some();
        while let Some(node) = self.worklist.pop() {
            if !self.graph.nodes[node].reachable {
                continue;
            }
            let produced = produce::Producer {
                evidence: self.evidence(),
                bindings: &self.bindings,
                heap: &self.heap,
            }
            .compute(node);
            if !contextual {
                self.update(node, produced);
                continue;
            }
            let mut facts = produced.clone();
            if facts.unknown && !self.required[node].arrays.is_empty() && self.heap.is_local(node) {
                facts.unknown = false;
            }
            self.update(node, require(facts, &self.required[node]));
            self.node_uses(node);
            for usage in self.uses[node].clone() {
                self.sink_use(usage);
            }
            // A producer join with neutral literal arms supplies its own
            // evidence to those arms. Opaque alternatives never do.
            if matches!(
                self.graph.nodes[node].op,
                Op::Read { .. } | Op::Select { .. }
            ) && !produced.unknown
            {
                self.demand(node, produced.clone());
            }
            if !self.required[node].is_empty() || !self.packing_layouts[node].is_empty() {
                self.backward(node, produced);
            }
        }
    }

    fn update(&mut self, node: NodeId, facts: Facts) {
        if self.facts[node] != facts {
            self.facts[node] = facts;
            for &reader in &self.readers[node] {
                self.worklist.enqueue(reader);
            }
        }
    }

    fn node_uses(&mut self, node: NodeId) {
        let op = self.graph.nodes[node].op.clone();
        match op {
            Op::Call {
                method,
                receiver,
                args,
                offset,
            } => self.call_uses(node, &method, receiver.value(), &args, offset),
            Op::ArrayLoad { array, index } => self.index_use(array, index),
            Op::Binary {
                op: BinaryOp::And | BinaryOp::Or | BinaryOp::Xor,
                left,
                right,
            } => {
                let source = self
                    .evidence()
                    .at(left, node)
                    .joined(&self.evidence().at(right, node));
                let flags = self.flags(&source);
                if !flags.domains.is_empty() {
                    self.demand(left, flags.clone());
                    self.demand(right, flags);
                }
            }
            _ => {}
        }
    }
    fn apply_contract(&mut self, node: NodeId, contract: &Contract, args: &[NodeId], at: NodeId) {
        self.demand(node, shape::contract(contract, &self.graph.nodes[node].ty));
        match &contract.derived {
            Some(DerivedDomain::Slot {
                parameter,
                dimension,
                slot,
            }) => {
                if let Some(&table) = args.get(*parameter) {
                    self.demand(
                        node,
                        shape::column(
                            self.bindings.mappings,
                            self.evidence().at(table, at),
                            *dimension,
                            *slot,
                        ),
                    );
                }
            }
            Some(DerivedDomain::Conditional(cases)) => {
                if let Some(&selector) = cases.first().and_then(|case| args.get(case.parameter))
                    && let Some(domain) = self.evidence().conditional(at, selector, cases)
                {
                    self.demand(node, Facts::scalar(domain));
                }
            }
            _ => {}
        }
    }
    fn call_uses(
        &mut self,
        node: NodeId,
        method: &Member,
        receiver: Option<NodeId>,
        args: &[NodeId],
        offset: u32,
    ) {
        for (index, &argument) in args.iter().enumerate() {
            if let Some(domain) =
                self.bindings
                    .mappings
                    .call_domain(self.bindings.member, offset, Some(index))
            {
                self.demand(argument, Facts::scalar(domain));
                continue;
            }
            let contract = self.bindings.method(method, Some(index));
            self.apply_contract(argument, &contract, args, argument);
            if !contract.class_name && self.graph.nodes[argument].ty.is_reference() {
                self.ordinary.insert(argument);
            }
        }
        if let Some(receiver) = receiver {
            // Receiver calls can observe string contents too, including through
            // an Object alias (length, hashCode, toString, equals, ...).
            self.ordinary.insert(receiver);
            let facts = self.evidence().final_at(receiver, node);
            if !facts.unknown {
                for container in &facts.containers {
                    self.container_uses(method, args, container);
                }
            }
            if method.owner == "java/lang/String"
                && args.len() == 1
                && matches!(
                    method.name.as_str(),
                    "equals" | "equalsIgnoreCase" | "compareTo"
                )
            {
                let mut left = self.evidence().final_at(receiver, node);
                let mut right = self.evidence().final_at(args[0], node);
                left.domains.retain(|&domain| {
                    self.bindings.mappings.domain(domain).kind == DomainKind::String
                });
                right.domains.retain(|&domain| {
                    self.bindings.mappings.domain(domain).kind == DomainKind::String
                });
                if !left.domains.is_empty() {
                    self.demand(args[0], left);
                }
                if !right.domains.is_empty() {
                    self.demand(receiver, right);
                }
            }
        }
    }
    fn container_uses(&mut self, method: &Member, args: &[NodeId], shape: &ContainerContract) {
        let Some(&first) = args.first() else {
            return;
        };
        if method.owner == "java/util/Vector" {
            if matches!(
                method.name.as_str(),
                "addElement"
                    | "insertElementAt"
                    | "setElementAt"
                    | "contains"
                    | "indexOf"
                    | "lastIndexOf"
                    | "removeElement"
            ) && let Some(domain) = shape.elements
            {
                self.demand(first, Facts::scalar(domain));
            }
            if method.name == "copyInto"
                && let Some(domain) = shape.elements
            {
                self.demand(
                    first,
                    Facts {
                        arrays: HashSet::from([Contract {
                            leaf: Some(domain),
                            ..Contract::default()
                        }]),
                        ..Facts::default()
                    },
                );
            }
        } else if method.owner == "java/util/Hashtable" {
            if matches!(
                method.name.as_str(),
                "get" | "put" | "remove" | "containsKey"
            ) && let Some(domain) = shape.keys
            {
                self.demand(first, Facts::scalar(domain));
            }
            if method.name == "put"
                && args.len() == 2
                && let Some(domain) = shape.values
            {
                self.demand(args[1], Facts::scalar(domain));
            }
            if method.name == "contains"
                && let Some(domain) = shape.values
            {
                self.demand(first, Facts::scalar(domain));
            }
        }
    }

    fn sink_use(&mut self, index: usize) {
        match self.graph.sinks[index].clone() {
            Sink::Field { field, value, .. } => {
                let contract = self.bindings.field(&field);
                self.demand(
                    value,
                    shape::contract(&contract, &self.graph.nodes[value].ty),
                );
                if !contract.class_name && self.graph.nodes[value].ty.is_reference() {
                    self.ordinary.insert(value);
                }
            }
            Sink::Store {
                array,
                index,
                value,
            } => {
                self.index_use(array, index);
                let source = self.evidence().at(array, value);
                let mut requested = Facts {
                    unknown: source.unknown,
                    ..Facts::default()
                };
                for shape in &source.arrays {
                    requested.merge(&shape::project(
                        self.bindings.mappings,
                        &self.ranges,
                        shape,
                        index,
                        &self.graph.nodes[value].ty,
                    ));
                }
                if !source.arrays.is_empty() {
                    self.demand(value, requested);
                }
                if self.graph.nodes[value].ty.is_reference()
                    && source.arrays.iter().all(|shape| !shape.class_name)
                {
                    self.ordinary.insert(value);
                }
            }
            Sink::Return(value) => {
                let contract = self
                    .bindings
                    .mappings
                    .contract(Target::Return(self.bindings.member))
                    .cloned()
                    .unwrap_or_default();
                self.demand(
                    value,
                    shape::contract(&contract, &self.graph.nodes[value].ty),
                );
                if let Some(DerivedDomain::Conditional(cases)) = &contract.derived
                    && let Some(&selector) = cases
                        .first()
                        .and_then(|case| self.graph.parameters.get(&case.parameter))
                    && let Some(domain) = self.evidence().conditional(value, selector, cases)
                {
                    self.demand(value, Facts::scalar(domain));
                }
                if !contract.class_name && self.graph.nodes[value].ty.is_reference() {
                    self.ordinary.insert(value);
                }
            }
            Sink::Compare { left, right, op } => {
                for (value, peer) in [(left, right), (right, left)] {
                    if matches!(self.graph.nodes[value].ty, JvmType::Object(_))
                        && !matches!(
                            self.graph.nodes[peer].op,
                            Op::Literal(crate::ir::Literal::Null)
                        )
                    {
                        self.ordinary.insert(value);
                    }
                    if !matches!(op, CompareOp::Eq | CompareOp::Ne)
                        && self.graph.integer(value) == Some(0)
                    {
                        continue;
                    }
                    let facts = self.evidence().final_at(peer, value);
                    // An opaque operand can receive a contextual domain from
                    // this comparison, but that does not make it evidence for
                    // the other operand. Feeding its unknownness back would
                    // erase an independently bound choice's literal names.
                    let producer = &self.producer()[peer];
                    if facts.unknown && producer.domains.is_empty() && producer.dependent.is_empty()
                    {
                        continue;
                    }
                    if !facts.domains.is_empty() {
                        self.demand(
                            value,
                            Facts {
                                domains: facts.domains,
                                unknown: facts.unknown,
                                ..Facts::default()
                            },
                        );
                    }
                }
            }
            Sink::Observe(value) => {
                self.ordinary.insert(value);
            }
            Sink::Switch { .. } | Sink::Discard(_) => {}
        }
    }

    fn index_use(&mut self, array: NodeId, index: NodeId) {
        let source = self.evidence().at(array, index);
        let mut domains = HashSet::new();
        let mut unknown = source.unknown;
        for shape in &source.arrays {
            let Some(binding) = shape.arrays.get(&0) else {
                unknown = true;
                continue;
            };
            let mut index_domain = binding
                .index
                .or(binding.slots.filter(|_| binding.records.is_none()));
            if let Some(layout) = &binding.records {
                if self
                    .graph
                    .integer(index)
                    .is_some_and(|index| index >= 0 && index < i64::from(layout.offset))
                {
                    index_domain = binding.slots;
                }
                let ambiguous = source.unknown
                    || source.arrays.iter().any(|other| {
                        other
                            .arrays
                            .get(&0)
                            .and_then(|dimension| dimension.records.as_ref())
                            != Some(layout)
                    });
                if let Some(slot) = ranges::slot(&self.ranges, index, layout)
                    && let Some(literal) = ranges::offset_literal(&self.ranges, index, layout, slot)
                {
                    self.demand(
                        literal,
                        Facts {
                            unknown: ambiguous,
                            ..Facts::scalar(layout.domain)
                        },
                    );
                }
                if layout.planes {
                    for (literal, slot, addend) in ranges::plane_offsets(self.graph, index, layout)
                    {
                        self.offsets
                            .entry(literal)
                            .or_default()
                            .insert((!ambiguous).then_some(OffsetConstant {
                                domain: layout.domain,
                                slot,
                                multiplier: i64::from(layout.stride),
                                addend,
                            }));
                    }
                }
            }
            domains.extend(index_domain);
            unknown |= index_domain.is_none();
        }
        if source.arrays.is_empty() {
            unknown = true;
        }
        if !domains.is_empty() {
            self.demand(
                index,
                Facts {
                    domains,
                    unknown,
                    ..Facts::default()
                },
            );
        }
    }

    fn flags(&self, facts: &Facts) -> Facts {
        Facts {
            domains: facts
                .domains
                .iter()
                .copied()
                .filter(|&domain| self.bindings.mappings.domain(domain).kind == DomainKind::Flags)
                .collect(),
            unknown: facts.unknown,
            ..Facts::default()
        }
    }
    fn backward(&mut self, node: NodeId, produced: Facts) {
        if self.ranges.induction[node].is_some() {
            return;
        }
        let required =
            self.evidence()
                .resolve(node, require(produced.clone(), &self.required[node]), false);
        // An encoded OR is a collection of packing terms. Project each term
        // separately; the whole word must never acquire the low field's domain.
        let mut packed_domains = self.packing_layouts[node].clone();
        packed_domains.extend(
            required
                .domains
                .iter()
                .copied()
                .filter(|&domain| self.bindings.mappings.domain(domain).kind == DomainKind::Packed),
        );
        if !matches!(
            self.graph.nodes[node].op,
            Op::Binary {
                op: BinaryOp::Or,
                ..
            }
        ) {
            for domain in &packed_domains {
                for field in &self.bindings.mappings.domain(*domain).bit_fields {
                    if field.selector_mask != 0 {
                        continue;
                    }
                    if let Some(packing) = packed::packing(self.graph, node, field)
                        && (packing.value != node || !self.packing_layouts[node].is_empty())
                        && let Some(domain) = field.domain
                    {
                        self.demand_source(packing.value, Facts::scalar(domain));
                    }
                }
            }
        }
        match self.graph.nodes[node].op.clone() {
            Op::Read { sources, .. } => {
                if produced.unknown && !self.heap.is_local(node) {
                    // Invalidate earlier contextual evidence if a later fixed
                    // point exposes an opaque reaching alternative.
                    for source in sources {
                        if !self.required[source].is_empty() {
                            self.demand(source, Facts::unknown());
                        }
                    }
                    return;
                }
                for source in sources {
                    self.demand_source(source, required.clone());
                }
            }
            Op::Copy(value) => self.demand_source(value, required),
            Op::Select { yes, no, .. } => {
                self.demand_source(yes, required.clone());
                self.demand_source(no, required);
            }
            Op::Cast { value, ty } => {
                if shape::cast_preserves(&self.graph.nodes[value].ty, &ty)
                    || shape::named_cast_preserves(&self.ranges, value, &ty)
                    || required
                        .domains
                        .iter()
                        .all(|&domain| shape::fits(self.bindings.mappings, domain, &ty))
                {
                    self.demand_source(value, required);
                }
            }
            Op::Binary { op, left, right } => {
                if matches!(op, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor) {
                    let flags = self.flags(&required);
                    self.demand(left, flags.clone());
                    self.demand(right, flags);
                }
                // OR combines separately packed fields, but never licenses
                // arbitrary arithmetic to inherit the whole word's meaning.
                if op == BinaryOp::Or && !packed_domains.is_empty() {
                    for child in [left, right] {
                        let before = self.packing_layouts[child].len();
                        self.packing_layouts[child].extend(&packed_domains);
                        if self.packing_layouts[child].len() != before {
                            self.worklist.enqueue(child);
                        }
                    }
                }
            }
            Op::Array { elements } => {
                let JvmType::Array(element_type) = &self.graph.nodes[node].ty else {
                    return;
                };
                let element_type = *element_type.clone();
                for shape in &required.arrays {
                    for (index, &value) in elements.iter().enumerate() {
                        self.demand(
                            value,
                            shape::project_constant(
                                self.bindings.mappings,
                                shape,
                                index as i64,
                                &element_type,
                            ),
                        );
                    }
                    for (index, value) in self.heap.stores(node).to_vec() {
                        self.demand(
                            value,
                            shape::project(
                                self.bindings.mappings,
                                &self.ranges,
                                shape,
                                index,
                                &self.graph.nodes[value].ty,
                            ),
                        );
                    }
                }
            }
            Op::Call {
                method,
                receiver,
                args,
                ..
            } => {
                if is_wrapper(&method.owner)
                    && (receiver == Receiver::New || method.name == "valueOf")
                    && args.len() == 1
                {
                    self.demand(args[0], required);
                } else if receiver == Receiver::New {
                    if is_container(&method.owner) {
                        for caller in self.heap.calls(node).to_vec() {
                            let Op::Call { method, args, .. } = self.graph.nodes[caller].op.clone()
                            else {
                                continue;
                            };
                            for shape in &required.containers {
                                self.container_uses(&method, &args, shape);
                            }
                        }
                    }
                } else if is_unboxing(&method) {
                    if let Some(receiver) = receiver.value()
                        && (wrapper_type(&method.owner).as_ref()
                            == Some(&self.graph.nodes[node].ty)
                            || required.domains.iter().all(|&domain| {
                                shape::fits(
                                    self.bindings.mappings,
                                    domain,
                                    &self.graph.nodes[node].ty,
                                )
                            }))
                    {
                        self.demand(receiver, required);
                    }
                } else if let Some(DerivedDomain::Parameter(index)) =
                    self.bindings.method(&method, None).derived
                    && let Some(&argument) = args.get(index)
                {
                    self.demand(argument, required);
                }
            }
            _ => {}
        }
    }
}

fn operands(op: &Op) -> Vec<NodeId> {
    match op {
        Op::Read { sources, .. } => sources.iter().copied().collect(),
        Op::Copy(value) | Op::Cast { value, .. } | Op::ArrayLength(value) => vec![*value],
        Op::Select {
            left,
            right,
            yes,
            no,
            ..
        } => vec![*left, *right, *yes, *no],
        Op::Binary { left, right, .. } => vec![*left, *right],
        Op::ArrayLoad { array, index } => vec![*array, *index],
        Op::Call { receiver, args, .. } => receiver
            .value()
            .iter()
            .copied()
            .chain(args.iter().copied())
            .collect(),
        Op::Array { elements } => elements.clone(),
        _ => Vec::new(),
    }
}
fn sink_operands(sink: &Sink) -> Vec<NodeId> {
    match sink {
        Sink::Field { value, .. }
        | Sink::Return(value)
        | Sink::Switch { value, .. }
        | Sink::Observe(value)
        | Sink::Discard(value) => vec![*value],
        Sink::Store {
            array,
            index,
            value,
        } => vec![*array, *index, *value],
        Sink::Compare { left, right, .. } => vec![*left, *right],
    }
}
