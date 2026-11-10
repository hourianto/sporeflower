//! Immutable may-alias and publication analysis for local arrays and containers.
//! Its indexes are built before domain inference and never absorb contextual facts.
use super::super::flow::{Graph, NodeId, Op, Receiver, Sink};
use super::{bindings::Bindings, shape::has_array};
use crate::hash::HashSet;
use crate::ir::{Literal, Member};
use crate::mappings::Target;
use std::collections::BTreeSet;

#[derive(Clone, Default, PartialEq, Eq)]
struct Origins {
    allocations: BTreeSet<NodeId>,
    external: bool,
}
impl Origins {
    fn join(&mut self, other: &Self) {
        self.allocations.extend(&other.allocations);
        self.external |= other.external;
    }
}

pub(super) struct Heap {
    stores: Vec<Vec<(NodeId, NodeId)>>,
    calls: Vec<Vec<NodeId>>,
    escaped: HashSet<NodeId>,
    opaque: HashSet<NodeId>,
    local: Vec<bool>,
    unpublished: Vec<bool>,
}

impl Heap {
    pub(super) fn analyze(graph: &Graph, bindings: &Bindings<'_, '_>) -> Self {
        let count = graph.nodes.len();
        let mut origins = vec![Origins::default(); count];
        let mut stores = vec![Vec::new(); count];
        let mut escaped = HashSet::default();
        let mut opaque = HashSet::default();
        let mut published = HashSet::<NodeId>::default();
        // Reference provenance includes Object casts and local array holders.
        // It is a may-analysis: any unknown origin or published nested object
        // remains visible rather than being hidden by a convenient alias.
        let array_writes: Vec<_> = graph
            .sinks
            .iter()
            .filter_map(|sink| match sink {
                Sink::Store { array, value, .. } => Some((*array, *value)),
                _ => None,
            })
            .collect();
        for (node, data) in graph.nodes.iter().enumerate() {
            if matches!(data.op, Op::Unknown) {
                origins[node].external = true;
            }
        }
        loop {
            let mut changed = false;
            let mut contents = vec![Origins::default(); graph.nodes.len()];
            for (node, data) in graph.nodes.iter().enumerate() {
                if let Op::Array { elements } = &data.op {
                    for &element in elements {
                        contents[node].join(&origins[element]);
                    }
                }
            }
            for &(array, value) in &array_writes {
                for &allocation in &origins[array].allocations {
                    contents[allocation].join(&origins[value]);
                }
            }
            for node in 0..graph.nodes.len() {
                if !graph.nodes[node].ty.is_reference() {
                    continue;
                }
                let mut incoming = Origins::default();
                match &graph.nodes[node].op {
                    Op::Array { .. } => {
                        incoming.allocations.insert(node);
                    }
                    Op::Call {
                        method,
                        receiver: Receiver::New,
                        ..
                    } if is_container(&method.owner) => {
                        incoming.allocations.insert(node);
                    }
                    Op::Literal(Literal::Null) => {}
                    Op::Copy(value) | Op::Cast { value, .. } => incoming.join(&origins[*value]),
                    Op::Read { sources, .. } => {
                        for &source in sources {
                            incoming.join(&origins[source]);
                        }
                    }
                    Op::Select { yes, no, .. } => {
                        incoming.join(&origins[*yes]);
                        incoming.join(&origins[*no]);
                    }
                    Op::ArrayLoad { array, .. } => {
                        incoming.external = origins[*array].external;
                        for &allocation in &origins[*array].allocations {
                            incoming.join(&contents[allocation]);
                        }
                    }
                    _ => incoming.external = true,
                }
                let before = origins[node].clone();
                origins[node].join(&incoming);
                changed |= origins[node] != before;
            }
            if !changed {
                break;
            }
        }
        let mut contents: Vec<BTreeSet<NodeId>> = vec![BTreeSet::new(); graph.nodes.len()];
        let mut published_contents = BTreeSet::<NodeId>::new();
        for (node, data) in graph.nodes.iter().enumerate() {
            if let Op::Array { elements } = &data.op {
                for &element in elements {
                    contents[node].extend(&origins[element].allocations);
                }
            }
            if let Op::Call { method, args, .. } = &data.op {
                // A contract describes element meanings, not all possible
                // contents after another method observes this allocation.
                for argument in args {
                    published.extend(&origins[*argument].allocations);
                }
                if is_array_copy(method) && args.len() == 5 {
                    opaque.extend(&origins[args[2]].allocations);
                    published_contents.extend(&origins[args[0]].allocations);
                } else {
                    for (index, &argument) in args.iter().enumerate() {
                        let contract = bindings.method(method, Some(index));
                        for &allocation in &origins[argument].allocations {
                            let array = matches!(graph.nodes[allocation].op, Op::Array { .. });
                            if array && !has_array(&contract, &graph.nodes[argument].ty)
                                || !array && contract.container.is_none()
                            {
                                escaped.insert(allocation);
                            }
                        }
                    }
                }
            }
        }
        for sink in &graph.sinks {
            match sink {
                Sink::Store {
                    array,
                    index,
                    value,
                } => {
                    if origins[*array].external {
                        published.extend(&origins[*value].allocations);
                    }
                    for &allocation in &origins[*array].allocations {
                        stores[allocation].push((*index, *value));
                        contents[allocation].extend(&origins[*value].allocations);
                    }
                    // A bound outer array is handled later as a schema boundary.
                    // Unannotated external holders cannot establish a local layout.
                    if origins[*array].external
                        && !explicit_array(graph, bindings, *array, &mut HashSet::default())
                    {
                        escaped.extend(&origins[*value].allocations);
                    }
                }
                Sink::Field { field, value, .. } => {
                    published.extend(&origins[*value].allocations);
                    let contract = bindings.field(field);
                    if !has_array(&contract, &graph.nodes[*value].ty)
                        && contract.container.is_none()
                    {
                        escaped.extend(&origins[*value].allocations);
                    }
                }
                Sink::Return(value) => published.extend(&origins[*value].allocations),
                _ => {}
            }
        }
        for allocation in published_contents {
            escaped.extend(&contents[allocation]);
        }
        loop {
            let old = escaped.len();
            let published_before = published.len();
            for &holder in &escaped.clone() {
                escaped.extend(&contents[holder]);
            }
            for &holder in &published.clone() {
                published.extend(&contents[holder]);
            }
            if escaped.len() == old && published.len() == published_before {
                break;
            }
        }
        let local = origins
            .iter()
            .map(|origin| {
                !origin.external
                    && !origin.allocations.is_empty()
                    && origin
                        .allocations
                        .iter()
                        .all(|allocation| !escaped.contains(allocation))
            })
            .collect();
        let unpublished = origins
            .iter()
            .map(|origin| {
                !origin.external
                    && !origin.allocations.is_empty()
                    && origin
                        .allocations
                        .iter()
                        .all(|allocation| !published.contains(allocation))
            })
            .collect();
        let mut calls = vec![Vec::new(); count];
        for (caller, data) in graph.nodes.iter().enumerate() {
            if let Op::Call {
                receiver: Receiver::Value(receiver),
                ..
            } = &data.op
            {
                for &allocation in &origins[*receiver].allocations {
                    calls[allocation].push(caller);
                }
            }
        }
        Self {
            stores,
            calls,
            escaped,
            opaque,
            local,
            unpublished,
        }
    }

    pub(super) fn stores(&self, allocation: NodeId) -> &[(NodeId, NodeId)] {
        &self.stores[allocation]
    }
    pub(super) fn calls(&self, allocation: NodeId) -> &[NodeId] {
        &self.calls[allocation]
    }
    pub(super) fn escaped(&self, allocation: NodeId) -> bool {
        self.escaped.contains(&allocation)
    }
    pub(super) fn opaque(&self, allocation: NodeId) -> bool {
        self.opaque.contains(&allocation)
    }
    pub(super) fn is_local(&self, node: NodeId) -> bool {
        self.local[node]
    }
    /// Literal enumeration needs stronger ownership than semantic propagation:
    /// even a fully annotated publication can replace elements elsewhere.
    pub(super) fn is_unpublished(&self, node: NodeId) -> bool {
        self.unpublished[node]
    }
}

fn explicit_array(
    graph: &Graph,
    bindings: &Bindings<'_, '_>,
    node: NodeId,
    visiting: &mut HashSet<NodeId>,
) -> bool {
    if !visiting.insert(node) {
        return false;
    }
    let result = match &graph.nodes[node].op {
        Op::Field(field) => has_array(&bindings.field(field), &graph.nodes[node].ty),
        Op::Parameter(index) => bindings
            .mappings
            .contract(Target::Parameter {
                method: bindings.member,
                index: *index,
            })
            .is_some_and(|contract| has_array(contract, &graph.nodes[node].ty)),
        Op::Call { method, .. } => has_array(&bindings.method(method, None), &graph.nodes[node].ty),
        Op::Copy(value) | Op::Cast { value, .. } => {
            explicit_array(graph, bindings, *value, visiting)
        }
        Op::Read { sources, .. } => {
            !sources.is_empty()
                && sources
                    .iter()
                    .all(|&source| explicit_array(graph, bindings, source, visiting))
        }
        Op::ArrayLoad { array, .. } => explicit_array(graph, bindings, *array, visiting),
        _ => false,
    };
    visiting.remove(&node);
    result
}

pub(super) fn is_container(owner: &str) -> bool {
    matches!(owner, "java/util/Vector" | "java/util/Hashtable")
}
fn is_array_copy(method: &Member) -> bool {
    method.owner == "java/lang/System"
        && method.name == "arraycopy"
        && method.descriptor == "(Ljava/lang/Object;ILjava/lang/Object;II)V"
}
