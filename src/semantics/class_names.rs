//! Class-name relocation plans retain original invocation identities. Shared
//! literals stay ordinary text; only their recognized consuming arguments adapt.
mod consumers;
pub(crate) use consumers::Consumers;

use super::{
    Analysis, ClassNameDecision, ClassNameDestination as Boundary, ClassNameGroup, ClassNameInput,
    flow::{Graph, NodeId, Op, Sink},
};
use crate::hash::HashSet;
use crate::{
    classfile::JvmType,
    ir::{Literal, Member, MethodIr, ValueId},
    mappings::{MappingPlan, Target},
    naming::NamePlan,
    program::{MemberId, Program},
};
use std::collections::{BTreeMap, BTreeSet};

pub fn is_for_name(method: &Member) -> bool {
    method.owner == "java/lang/Class"
        && method.name == "forName"
        && matches!(
            method.descriptor.as_str(),
            "(Ljava/lang/String;)Ljava/lang/Class;"
                | "(Ljava/lang/String;ZLjava/lang/ClassLoader;)Ljava/lang/Class;"
        )
}

/// Resolve contracts before any owner, method, or parameter is renamed.
pub fn is_argument(
    program: &Program<'_>,
    mappings: &MappingPlan,
    method: &Member,
    index: usize,
) -> bool {
    is_for_name(method) && index == 0
        || program
            .class_name_consumers()
            .is_argument(program, method, index)
        || program
            .resolve_method(&method.owner, &method.name, &method.descriptor)
            .and_then(|method| mappings.contract(Target::Parameter { method, index }))
            .is_some_and(|contract| contract.class_name)
}

/// Class.forName accepts binary names, including object array descriptors.
/// An internal slash spelling must retain its original lookup failure.
pub fn relocated(original: &str, names: &NamePlan) -> Option<String> {
    if original.contains('/') {
        return None;
    }
    let internal = original.replace('.', "/");
    if internal.starts_with('[') {
        JvmType::parse(&internal)
            .ok()
            .map(|ty| descriptor(&ty, names).replace('/', "."))
    } else {
        Some(names.class_name_for(&internal).replace('/', "."))
    }
}

fn descriptor(ty: &JvmType, names: &NamePlan) -> String {
    match ty {
        JvmType::Object(name) => format!("L{};", names.class_name_for(name)),
        JvmType::Array(element) => format!("[{}", descriptor(element, names)),
        _ => ty.descriptor(),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "class-name planning combines method context, array queries, and the accumulated analysis"
)]
pub(super) fn plan<'a>(
    graph: &Graph,
    ir: &MethodIr,
    program: &Program<'_>,
    mappings: &MappingPlan,
    member: MemberId,
    array_stores: impl Fn(NodeId) -> (bool, &'a [(NodeId, NodeId)]),
    class_name_array: impl Fn(NodeId, NodeId) -> bool,
    mut shared: HashSet<ValueId>,
    analysis: &mut Analysis,
) {
    let mut inputs = BTreeMap::<Boundary, (BTreeMap<ValueId, String>, bool)>::new();
    for data in &graph.nodes {
        let Op::Call {
            method,
            args,
            offset,
            ..
        } = &data.op
        else {
            continue;
        };
        if !data.reachable {
            continue;
        }
        for (index, &argument) in args.iter().enumerate() {
            if mappings.call_domain(member, *offset, Some(index)).is_some()
                || !is_argument(program, mappings, method, index)
            {
                continue;
            }
            let boundary = if graph.nodes[argument].ty == JvmType::Object("java/lang/String".into())
            {
                Boundary::Argument {
                    offset: *offset,
                    parameter: index,
                }
            } else {
                Boundary::ArrayArgument {
                    offset: *offset,
                    parameter: index,
                }
            };
            let (names, complete) = inputs
                .entry(boundary)
                .or_insert_with(|| (BTreeMap::new(), true));
            *complete &= literal_sources(
                graph,
                argument,
                &array_stores,
                names,
                &mut HashSet::default(),
            );
        }
    }
    let mut non_scalar = Vec::new();
    for sink in &graph.sinks {
        let (value, boundary) = match sink {
            Sink::Field { field, value, pc }
                if program
                    .resolve_field(&field.owner, &field.name, &field.descriptor)
                    .and_then(|field| mappings.contract(Target::Field(field)))
                    .is_some_and(|contract| contract.class_name) =>
            {
                let boundary = (field.descriptor == "Ljava/lang/String;")
                    .then_some(*pc)
                    .flatten()
                    .map(|offset| Boundary::Field { offset });
                (*value, boundary)
            }
            Sink::Return(value)
                if mappings
                    .contract(Target::Return(member))
                    .is_some_and(|contract| contract.class_name) =>
            {
                (*value, None)
            }
            Sink::Store { array, value, .. } if class_name_array(*array, *value) => (*value, None),
            _ => continue,
        };
        if !graph.nodes[value].reachable {
            continue;
        }
        if let Some(boundary) = boundary {
            let (names, complete) = inputs
                .entry(boundary)
                .or_insert_with(|| (BTreeMap::new(), true));
            *complete &=
                literal_sources(graph, value, &array_stores, names, &mut HashSet::default());
        } else {
            let mut names = BTreeMap::new();
            let complete = literal_sources(
                graph,
                value,
                &array_stores,
                &mut names,
                &mut HashSet::default(),
            );
            let destination = match sink {
                Sink::Return(_) => Boundary::Return,
                Sink::Store { .. } => Boundary::ArrayStore,
                _ => Boundary::NonScalarField,
            };
            non_scalar.push((destination, (names, complete)));
        }
    }
    // Local array loads need not have propagated class-name demand to their
    // elements. Their unchanged finite literals still need consumer adaptation.
    for (names, complete) in inputs.values() {
        if *complete {
            shared.extend(
                names
                    .keys()
                    .copied()
                    .filter(|value| !analysis.class_names.contains(value)),
            );
        }
    }
    // Build connected components once. Both source emission and native width
    // planning consume these components; neither repeats a closure over calls.
    let inputs: Vec<_> = inputs.into_iter().chain(non_scalar).collect();
    let mut readers = BTreeMap::<ValueId, Vec<usize>>::new();
    for (index, (_, (names, _))) in inputs.iter().enumerate() {
        for &value in names.keys() {
            readers.entry(value).or_default().push(index);
        }
    }
    let mut visited = HashSet::default();
    for start in 0..inputs.len() {
        if !visited.insert(start) || inputs[start].1.0.is_empty() {
            continue;
        }
        let mut pending = vec![start];
        let mut members = Vec::new();
        let mut origins = BTreeSet::new();
        while let Some(index) = pending.pop() {
            members.push(index);
            for &value in inputs[index].1.0.keys() {
                if origins.insert(value) {
                    for &reader in &readers[&value] {
                        if visited.insert(reader) {
                            pending.push(reader);
                        }
                    }
                }
            }
        }
        members.sort_unstable();
        let preserve = origins.iter().any(|value| shared.contains(value));
        if preserve {
            shared.extend(&origins);
        }
        let mut group = ClassNameGroup {
            origins: origins.iter().map(|&value| ir.values[value].pc).collect(),
            inputs: members
                .into_iter()
                .map(|index| {
                    let (destination, (names, complete)) = &inputs[index];
                    ClassNameInput {
                        destination: *destination,
                        names: names
                            .values()
                            .cloned()
                            .collect::<BTreeSet<_>>()
                            .into_iter()
                            .collect(),
                        origins: names
                            .keys()
                            .map(|&value| ir.values[value].pc)
                            .collect::<BTreeSet<_>>()
                            .into_iter()
                            .collect(),
                        complete: *complete,
                    }
                })
                .collect(),
            decision: ClassNameDecision::RewriteProducers,
        };
        if preserve {
            group.require_adapter();
        }
        analysis.class_name_groups.push(group);
    }
    analysis.class_names.retain(|value| !shared.contains(value));
    analysis.shared_class_names = shared;
}

fn literal_sources<'a>(
    graph: &Graph,
    node: NodeId,
    array_stores: &impl Fn(NodeId) -> (bool, &'a [(NodeId, NodeId)]),
    names: &mut BTreeMap<ValueId, String>,
    seen: &mut HashSet<NodeId>,
) -> bool {
    if !seen.insert(node) {
        return true;
    }
    match &graph.nodes[node].op {
        Op::Literal(Literal::String(units)) => {
            if let Some(value) = graph.nodes[node].value
                && let Ok(text) = String::from_utf16(units)
            {
                names.insert(value, text);
                true
            } else {
                false
            }
        }
        Op::Literal(Literal::Null) => true,
        Op::Read { sources, .. } => {
            let mut complete = !sources.is_empty();
            for &source in sources {
                complete &= literal_sources(graph, source, array_stores, names, seen);
            }
            complete
        }
        Op::Copy(source) | Op::Cast { value: source, .. } => {
            literal_sources(graph, *source, array_stores, names, seen)
        }
        Op::Select { yes, no, .. } => {
            let yes = literal_sources(graph, *yes, array_stores, names, seen);
            let no = literal_sources(graph, *no, array_stores, names, seen);
            yes && no
        }
        Op::ArrayLoad { array, .. } => literal_sources(graph, *array, array_stores, names, seen),
        Op::Array { elements } => {
            let (mut complete, stores) = array_stores(node);
            for &element in elements.iter().chain(stores.iter().map(|(_, value)| value)) {
                complete &= literal_sources(graph, element, array_stores, names, seen);
            }
            complete
        }
        _ => false,
    }
}
