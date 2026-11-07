//! Infer lookup-only parameter contracts from original, exact method bodies.
//! A wrapper may forward a name through other wrappers, but observing or
//! publishing the same string makes caller-side relocation unsafe.
use super::is_for_name;
use crate::{
    bytecode::{Operand, decode},
    classfile::{JvmType, MethodDescriptor, PRIVATE, STATIC},
    ir::{Literal, Member},
    program::{MemberId, Program},
    semantics::flow::{Graph, NodeId, Op, Sink},
};
use std::collections::{BTreeMap, BTreeSet, HashSet};

type Parameter = (MemberId, usize);

#[derive(Default)]
pub(crate) struct Consumers(BTreeSet<Parameter>);

impl Consumers {
    pub fn is_argument(&self, program: &Program<'_>, method: &Member, index: usize) -> bool {
        program
            .resolve_method(&method.owner, &method.name, &method.descriptor)
            .is_some_and(|method| self.0.contains(&(method, index)))
    }

    pub fn has_arguments(&self, program: &Program<'_>, method: &Member) -> bool {
        program
            .resolve_method(&method.owner, &method.name, &method.descriptor)
            .is_some_and(|method| {
                self.0
                    .range((method, 0)..=(method, usize::MAX))
                    .next()
                    .is_some()
            })
    }

    pub fn build(program: &Program<'_>) -> Self {
        // Discover the reverse lookup-call closure from bytecode before lifting
        // bodies. Unrelated string-processing methods need no semantic graph.
        let mut callers = BTreeMap::<MemberId, Vec<MemberId>>::new();
        let mut pending = Vec::new();
        for class in 0..program.own_count() {
            let owner = program.class(class);
            for (index, method) in owner.methods.iter().enumerate() {
                // Virtual dispatch must not select an overriding implementation
                // that treats this argument as ordinary text.
                let exact = method.access_flags & (STATIC | PRIVATE | 0x0010) != 0
                    || owner.access_flags & 0x0010 != 0;
                if !exact
                    || method.name.starts_with('<')
                    || !MethodDescriptor::parse(&method.descriptor)
                        .is_ok_and(|descriptor| descriptor.parameters.iter().any(is_string))
                {
                    continue;
                }
                let Some(code) = &method.code else { continue };
                let Ok(instructions) = decode(&code.bytes) else {
                    continue;
                };
                let caller = MemberId { class, index };
                for instruction in instructions {
                    if !matches!(instruction.opcode, 0xb6..=0xb9) {
                        continue;
                    }
                    let index = match instruction.operand {
                        Operand::Constant(index) | Operand::InterfaceCall { index, .. } => index,
                        _ => continue,
                    };
                    let Ok(reference) = owner.constant_pool.member_ref(index) else {
                        continue;
                    };
                    let method = Member {
                        owner: reference.owner.into(),
                        name: reference.name.into(),
                        descriptor: reference.descriptor.into(),
                    };
                    if is_for_name(&method) {
                        pending.push(caller);
                    } else if let Some(callee) =
                        program.resolve_method(&method.owner, &method.name, &method.descriptor)
                    {
                        callers.entry(callee).or_default().push(caller);
                    }
                }
            }
        }
        let mut candidates = BTreeSet::new();
        while let Some(method) = pending.pop() {
            if candidates.insert(method) {
                pending.extend(callers.get(&method).into_iter().flatten().copied());
            }
        }
        let mut uses = BTreeMap::new();
        for method in candidates {
            let Ok(ir) = program.decompile_method(method, &[]) else {
                continue;
            };
            let graph = Graph::build(&ir);
            for (index, usage) in parameter_uses(&graph, program) {
                uses.insert((method, index), usage);
            }
        }
        // Contracts grow only from proven lookup consumers. Recursive forwarding
        // without a proven base case cannot establish a contract by itself.
        let mut result = Self::default();
        loop {
            let mut changed = false;
            for (&parameter, usage) in &uses {
                if !usage.ordinary
                    && (usage.lookup || !usage.forwarded.is_empty())
                    && usage
                        .forwarded
                        .iter()
                        .all(|callee| result.0.contains(callee))
                {
                    changed |= result.0.insert(parameter);
                }
            }
            if !changed {
                return result;
            }
        }
    }
}

fn is_string(ty: &JvmType) -> bool {
    matches!(ty, JvmType::Object(name) if name == "java/lang/String")
}

#[derive(Default)]
struct Uses {
    lookup: bool,
    ordinary: bool,
    forwarded: BTreeSet<Parameter>,
}

fn parameter_uses(graph: &Graph, program: &Program<'_>) -> BTreeMap<usize, Uses> {
    let mut uses: BTreeMap<_, _> = graph
        .parameters
        .iter()
        .filter(|(_, node)| is_string(&graph.nodes[**node].ty))
        .map(|(&index, _)| (index, Uses::default()))
        .collect();
    let mut record = |node: NodeId, callee: Option<Parameter>, lookup: bool| {
        let mut pending = vec![node];
        let mut seen = HashSet::new();
        while let Some(node) = pending.pop() {
            if !graph.nodes[node].reachable || !seen.insert(node) {
                continue;
            }
            match &graph.nodes[node].op {
                Op::Parameter(index) => {
                    if let Some(usage) = uses.get_mut(index) {
                        usage.lookup |= lookup;
                        if let Some(callee) = callee {
                            usage.forwarded.insert(callee);
                        } else if !lookup {
                            usage.ordinary = true;
                        }
                    }
                }
                Op::Read { sources, .. } => pending.extend(sources),
                Op::Copy(value) | Op::Cast { value, .. } => pending.push(*value),
                Op::Select { yes, no, .. } => pending.extend([*yes, *no]),
                _ => {}
            }
        }
    };
    let comparison = |left: NodeId, right: NodeId| {
        if matches!(graph.nodes[left].op, Op::Literal(Literal::Null))
            || matches!(graph.nodes[right].op, Op::Literal(Literal::Null))
        {
            Vec::new()
        } else {
            vec![left, right]
        }
    };
    for data in &graph.nodes {
        if !data.reachable {
            continue;
        }
        let ordinary = match &data.op {
            Op::Call {
                method,
                receiver,
                args,
                ..
            } => {
                let callee =
                    program.resolve_method(&method.owner, &method.name, &method.descriptor);
                for (index, &argument) in args.iter().enumerate() {
                    let lookup = is_for_name(method) && index == 0;
                    record(
                        argument,
                        callee.filter(|_| !lookup).map(|method| (method, index)),
                        lookup,
                    );
                }
                receiver.value().into_iter().collect()
            }
            Op::Select { left, right, .. } => comparison(*left, *right),
            Op::Binary { left, right, .. } => vec![*left, *right],
            Op::ArrayLoad { array, index } => vec![*array, *index],
            Op::ArrayLength(array) => vec![*array],
            Op::Array { elements } => elements.clone(),
            _ => Vec::new(),
        };
        for node in ordinary {
            record(node, None, false);
        }
    }
    for sink in &graph.sinks {
        let ordinary = match sink {
            Sink::Field { value, .. }
            | Sink::Return(value)
            | Sink::Observe(value)
            | Sink::Switch { value, .. } => vec![*value],
            Sink::Store {
                array,
                index,
                value,
            } => vec![*array, *index, *value],
            Sink::Compare { left, right, .. } => comparison(*left, *right),
            Sink::Discard(_) => Vec::new(),
        };
        for node in ordinary {
            record(node, None, false);
        }
    }
    uses
}
