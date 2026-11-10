//! Exact 0/1 facts for original JVM producers. Each method is lifted in isolation
//! and reduced to dependencies between fields, parameters and returns. Only
//! these compact summaries survive into the program-wide propagation graph.
use super::{BinaryOp, InvokeKind, Literal, Member, ValueId, ValueKind, lift::EventKind};
use crate::hash::{HashMap, HashSet};
use crate::{
    bytecode::{Operand, decode},
    classfile::{Constant, JvmType, MethodDescriptor, NATIVE, PRIVATE},
    program::{ClassId, MemberId, Program},
};
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Fact {
    Field(MemberId),
    Parameter(MemberId, usize),
    Return(MemberId),
    Dispatch(ClassId, MemberId),
}

#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct Summary {
    canonical: HashSet<Fact>,
}

/// The same classification drives both summary edges and source queries.
enum Evidence {
    Canonical,
    Depends(Fact),
    Unknown,
}

fn call_evidence(
    program: &Program<'_>,
    caller: Option<ClassId>,
    kind: InvokeKind,
    method: &Member,
) -> Evidence {
    if let Some(target) = program.exact_method(caller, kind, method) {
        if program.canonical_boolean_api_implementation(target) {
            return Evidence::Canonical;
        }
        if program.is_own(target.class) {
            if method.descriptor.ends_with(")Z") {
                return Evidence::Depends(Fact::Return(target));
            }
            if program
                .normal_int_return(method)
                .is_some_and(|value| matches!(value, 0 | 1))
            {
                return Evidence::Canonical;
            }
        }
        return Evidence::Unknown;
    }
    // These catalog bodies cannot be overridden even when --no-api omitted
    // their declarations. Other platform contracts certify bodies, not families.
    if kind == InvokeKind::Virtual
        && program
            .id(&method.owner)
            .is_none_or(|owner| !program.is_own(owner))
        && crate::api::ApiCatalog::bundled().is_ok_and(|api| api.final_boolean_return(method))
    {
        return Evidence::Canonical;
    }
    if matches!(kind, InvokeKind::Virtual | InvokeKind::Interface)
        && let (Some(owner), Some(target)) = (
            program.id(&method.owner),
            program.resolve_method(&method.owner, &method.name, &method.descriptor),
        )
        && method.descriptor.ends_with(")Z")
    {
        return Evidence::Depends(Fact::Dispatch(owner, target));
    }
    Evidence::Unknown
}

/// A platform predicate certifies its implementation only. A virtual result
/// also requires a closed receiver family and every concrete implementation.
fn dispatch_inputs(program: &Program<'_>, owner: ClassId, target: MemberId) -> Option<Vec<Fact>> {
    let targets = program.virtual_targets(owner, target);
    if !program.application_scope() || !targets.hierarchy_complete {
        return None;
    }
    let mut inputs = Vec::new();
    let mut concrete = false;
    for &target in &targets.members {
        if program.method(target).access_flags & 0x0400 != 0 {
            continue;
        }
        concrete = true;
        if program.is_own(target.class) {
            inputs.push(Fact::Return(target));
        } else if !program.canonical_boolean_api_implementation(target) {
            return None;
        }
    }
    concrete.then_some(inputs)
}

impl Summary {
    pub(crate) fn canonical_return(
        &self,
        program: &Program<'_>,
        caller: Option<ClassId>,
        kind: InvokeKind,
        method: &Member,
    ) -> bool {
        match call_evidence(program, caller, kind, method) {
            Evidence::Canonical => true,
            Evidence::Depends(Fact::Dispatch(owner, target)) => {
                dispatch_inputs(program, owner, target)
                    .is_some_and(|inputs| inputs.iter().all(|input| self.canonical.contains(input)))
            }
            Evidence::Depends(fact) => self.canonical.contains(&fact),
            Evidence::Unknown => false,
        }
    }
    pub(crate) fn field(&self, field: MemberId) -> bool {
        self.canonical.contains(&Fact::Field(field))
    }
    pub(crate) fn parameter(&self, method: MemberId, index: usize) -> bool {
        self.canonical.contains(&Fact::Parameter(method, index))
    }

    pub(crate) fn build(program: &Program<'_>) -> Self {
        if !has_boolean_facts(program) {
            return Self::default();
        }
        Self::finish(
            program,
            program
                .owned_methods()
                .map(|member| summarize(program, member))
                .collect(),
        )
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn build_parallel(program: &Program<'_>) -> Self {
        if !has_boolean_facts(program) {
            return Self::default();
        }
        let methods: Vec<_> = program.owned_methods().collect();
        Self::finish(
            program,
            methods
                .par_iter()
                .map(|&member| summarize(program, member))
                .collect(),
        )
    }
    fn finish(program: &Program<'_>, summaries: Vec<MethodSummary>) -> Self {
        let mut graph = Graph::default();
        for class in 0..program.own_count() {
            for (index, field) in program.class(class).fields.iter().enumerate() {
                if field.descriptor == "Z" {
                    graph.node(program, Fact::Field(MemberId { class, index }));
                }
            }
        }
        for member in program.owned_methods() {
            if let Ok(descriptor) = MethodDescriptor::parse(&program.method(member).descriptor) {
                if descriptor.return_type == JvmType::Boolean {
                    graph.node(program, Fact::Return(member));
                }
                for (index, ty) in descriptor.parameters.iter().enumerate() {
                    if *ty == JvmType::Boolean {
                        graph.node(program, Fact::Parameter(member, index));
                    }
                }
            }
        }
        let mut called = HashSet::default();
        for summary in summaries {
            for dependency in summary.dependencies {
                if let Fact::Parameter(member, _) = dependency.target {
                    called.insert(member);
                }
                let target = graph.node(program, dependency.target);
                graph.canonical[target] &= dependency.known;
                for input in dependency.inputs {
                    let input = graph.node(program, input);
                    graph.users[input].push(target);
                }
            }
        }
        for (&fact, &node) in &graph.nodes {
            if let Fact::Parameter(member, _) = fact
                && !called.contains(&member)
            {
                graph.canonical[node] = false;
            }
        }
        let mut pending: Vec<_> = graph
            .canonical
            .iter()
            .enumerate()
            .filter_map(|(node, &known)| (!known).then_some(node))
            .collect();
        while let Some(input) = pending.pop() {
            for &node in &graph.users[input] {
                if std::mem::replace(&mut graph.canonical[node], false) {
                    pending.push(node);
                }
            }
        }
        Self {
            canonical: graph
                .nodes
                .into_iter()
                .filter_map(|(fact, node)| graph.canonical[node].then_some(fact))
                .collect(),
        }
    }
}

fn has_boolean_facts(program: &Program<'_>) -> bool {
    (0..program.own_count()).any(|class| {
        let class = program.class(class);
        class.fields.iter().any(|field| field.descriptor == "Z")
            || class.methods.iter().any(|method| {
                MethodDescriptor::parse(&method.descriptor).is_ok_and(|descriptor| {
                    descriptor.return_type == JvmType::Boolean
                        || descriptor.parameters.contains(&JvmType::Boolean)
                })
            })
    })
}

#[derive(Default)]
struct Graph {
    nodes: HashMap<Fact, usize>,
    canonical: Vec<bool>,
    users: Vec<Vec<usize>>,
}
impl Graph {
    fn node(&mut self, program: &Program<'_>, fact: Fact) -> usize {
        if let Some(&node) = self.nodes.get(&fact) {
            return node;
        }
        let node = self.canonical.len();
        self.nodes.insert(fact, node);
        self.users.push(Vec::new());
        let known = match fact {
            Fact::Field(member) => {
                let field = program.field(member);
                (program.application_scope() && program.complete_hierarchy(member.class)
                    || field.access_flags & (PRIVATE | 0x0010) != 0)
                    && field.constant_value.is_none_or(|index| {
                        matches!(
                            program.class(member.class).constant_pool.get(index),
                            Ok(Constant::Integer(0 | 1))
                        )
                    })
            }
            Fact::Parameter(member, _) => {
                program.method(member).code.is_some() && !program.external_method_inputs(member)
            }
            Fact::Return(member) => program.method(member).code.is_some(),
            Fact::Dispatch(..) => true,
        };
        self.canonical.push(known);
        if let Fact::Dispatch(owner, target) = fact {
            if let Some(inputs) = dispatch_inputs(program, owner, target) {
                for input in inputs {
                    let input = self.node(program, input);
                    self.users[input].push(node);
                }
            } else {
                self.canonical[node] = false;
            }
        }
        node
    }
}

#[derive(Default)]
struct MethodSummary {
    dependencies: Vec<Dependency>,
}
struct Dependency {
    target: Fact,
    known: bool,
    inputs: Vec<Fact>,
}

fn arguments(
    program: &Program<'_>,
    caller: ClassId,
    kind: InvokeKind,
    method: &Member,
) -> Vec<(MemberId, usize)> {
    let Ok(descriptor) = MethodDescriptor::parse(&method.descriptor) else {
        return Vec::new();
    };
    if !descriptor.parameters.contains(&JvmType::Boolean) {
        return Vec::new();
    }
    program
        .call_targets(caller, kind, method)
        .into_iter()
        .filter(|target| program.is_own(target.class))
        .flat_map(|target| {
            descriptor
                .parameters
                .iter()
                .enumerate()
                .filter_map(move |(index, ty)| (*ty == JvmType::Boolean).then_some((target, index)))
        })
        .collect()
}

fn reference_effects(
    program: &Program<'_>,
    caller: ClassId,
    opcode: u8,
    member: &Member,
    affected: &mut HashSet<Fact>,
) {
    if matches!(opcode, 0xb3 | 0xb5) {
        if member.descriptor == "Z"
            && let Some(field) =
                program.resolve_field(&member.owner, &member.name, &member.descriptor)
            && program.is_own(field.class)
        {
            affected.insert(Fact::Field(field));
        }
    } else {
        let kind = match opcode {
            0xb6 => InvokeKind::Virtual,
            0xb7 => InvokeKind::Special,
            0xb8 => InvokeKind::Static,
            _ => InvokeKind::Interface,
        };
        affected.extend(
            arguments(program, caller, kind, member)
                .into_iter()
                .map(|(member, index)| Fact::Parameter(member, index)),
        );
    }
}

fn summarize(program: &Program<'_>, member: MemberId) -> MethodSummary {
    let owner = program.class(member.class);
    let method = program.method(member);
    let mut affected = HashSet::default();
    if method.descriptor.ends_with(")Z") {
        affected.insert(Fact::Return(member));
    }
    let unknown = |affected: HashSet<Fact>| MethodSummary {
        dependencies: affected
            .into_iter()
            .map(|target| Dependency {
                target,
                known: false,
                inputs: Vec::new(),
            })
            .collect(),
    };
    let Some(code) = &method.code else {
        if method.access_flags & NATIVE != 0 {
            // JNI can bypass Java access checks. Keep storage and call inputs
            // open, while independent local return proofs remain available.
            for class in 0..program.own_count() {
                for (index, field) in program.class(class).fields.iter().enumerate() {
                    if field.descriptor == "Z" {
                        affected.insert(Fact::Field(MemberId { class, index }));
                    }
                }
                for (index, target) in program.class(class).methods.iter().enumerate() {
                    if let Ok(descriptor) = MethodDescriptor::parse(&target.descriptor) {
                        affected.extend(descriptor.parameters.iter().enumerate().filter_map(
                            |(parameter, ty)| {
                                (*ty == JvmType::Boolean).then_some(Fact::Parameter(
                                    MemberId { class, index },
                                    parameter,
                                ))
                            },
                        ));
                    }
                }
            }
        }
        return unknown(affected);
    };
    let decoded = match decode(&code.bytes) {
        Ok(decoded) => decoded,
        Err(_) => {
            // A malformed stream cannot identify its writes precisely. Bound
            // the uncertainty by this class's symbolic references, not every
            // field and method in the application.
            for (index, constant) in owner.constant_pool.entries.iter().enumerate() {
                let opcodes: &[u8] = match constant {
                    Constant::Fieldref { .. } => &[0xb3],
                    Constant::Methodref { .. } | Constant::InterfaceMethodref { .. } => {
                        &[0xb6, 0xb7, 0xb8]
                    }
                    _ => continue,
                };
                if let Ok(reference) = owner.constant_pool.member_ref(index as u16) {
                    let referenced = Member {
                        owner: reference.owner.into(),
                        name: reference.name.into(),
                        descriptor: reference.descriptor.into(),
                    };
                    for &opcode in opcodes {
                        reference_effects(
                            program,
                            member.class,
                            opcode,
                            &referenced,
                            &mut affected,
                        );
                    }
                }
            }
            return unknown(affected);
        }
    };
    for instruction in &decoded {
        if !matches!(instruction.opcode, 0xb3 | 0xb5..=0xb9) {
            continue;
        }
        let index = match instruction.operand {
            Operand::Constant(index) | Operand::InterfaceCall { index, .. } => index,
            _ => continue,
        };
        if let Ok(reference) = owner.constant_pool.member_ref(index) {
            let referenced = Member {
                owner: reference.owner.into(),
                name: reference.name.into(),
                descriptor: reference.descriptor.into(),
            };
            reference_effects(
                program,
                member.class,
                instruction.opcode,
                &referenced,
                &mut affected,
            );
        }
    }
    if affected.is_empty() {
        return MethodSummary::default();
    }
    let raw = crate::cfg::PreparedCode::from_decoded(code, decoded).and_then(|prepared| {
        super::lift::lift(owner, method, &prepared.instructions, &prepared.graph)
    });
    let Ok(raw) = raw else {
        return unknown(affected);
    };
    let mut roots: HashMap<Fact, Vec<ValueId>> = HashMap::default();
    if method.descriptor.ends_with(")Z") {
        let returns = roots.entry(Fact::Return(member)).or_default();
        for block in &raw.ir.blocks {
            if let super::Terminator::Return(Some(value)) = block.terminator {
                returns.push(value);
            }
        }
    }
    let add_call =
        |roots: &mut HashMap<Fact, Vec<ValueId>>, kind, method: &Member, args: &[ValueId]| {
            for (target, index) in arguments(program, member.class, kind, method) {
                if let Some(&value) = args.get(index) {
                    roots
                        .entry(Fact::Parameter(target, index))
                        .or_default()
                        .push(value);
                }
            }
        };
    for value in &raw.ir.values {
        match &value.kind {
            ValueKind::Call {
                kind, method, args, ..
            } => add_call(&mut roots, *kind, method, args),
            ValueKind::New {
                class,
                descriptor,
                args,
                ..
            } => add_call(
                &mut roots,
                InvokeKind::Special,
                &Member {
                    owner: class.clone(),
                    name: "<init>".into(),
                    descriptor: descriptor.clone(),
                },
                args,
            ),
            _ => (),
        }
    }
    for events in &raw.events {
        for event in events {
            match &event.kind {
                EventKind::Action(super::Statement::PutField { field, value, .. }) => {
                    if field.descriptor == "Z"
                        && let Some(field) =
                            program.resolve_field(&field.owner, &field.name, &field.descriptor)
                        && program.is_own(field.class)
                    {
                        roots.entry(Fact::Field(field)).or_default().push(*value);
                    }
                }
                EventKind::Action(super::Statement::ConstructorCall { method, args, .. }) => {
                    add_call(&mut roots, InvokeKind::Special, method, args)
                }
                _ => (),
            }
        }
    }
    // Traverse only the backward slices of Boolean sinks. Generational marks
    // bound each sink to one visit per value without retaining local IR nodes.
    let mut seen = vec![0usize; raw.ir.values.len()];
    let mut dependencies = Vec::with_capacity(roots.len());
    for (generation, (target, mut pending)) in roots.into_iter().enumerate() {
        let mut known = !pending.is_empty();
        let mut inputs = HashSet::default();
        while let Some(value) = pending.pop() {
            if seen[value.index()] == generation + 1 {
                continue;
            }
            seen[value.index()] = generation + 1;
            let evidence = match &raw.ir.values[value].kind {
                ValueKind::Literal(Literal::Int(0 | 1)) | ValueKind::InstanceOf { .. } => {
                    Evidence::Canonical
                }
                ValueKind::Alias(input) => {
                    pending.push(*input);
                    Evidence::Canonical
                }
                ValueKind::Phi { inputs, .. } if !inputs.is_empty() => {
                    pending.extend(inputs.iter().map(|input| input.value));
                    Evidence::Canonical
                }
                ValueKind::Binary {
                    op: BinaryOp::And, ..
                } if super::type_facts::has_boolean_mask(
                    &raw.ir,
                    super::ValueUse::Definition(value),
                ) =>
                {
                    Evidence::Canonical
                }
                ValueKind::Binary {
                    op: BinaryOp::And | BinaryOp::Or | BinaryOp::Xor,
                    left,
                    right,
                } => {
                    pending.extend([left, right]);
                    Evidence::Canonical
                }
                ValueKind::Parameter {
                    index: Some(index), ..
                } if MethodDescriptor::parse(&method.descriptor).is_ok_and(|descriptor| {
                    descriptor.parameters.get(*index) == Some(&JvmType::Boolean)
                }) =>
                {
                    Evidence::Depends(Fact::Parameter(member, *index))
                }
                ValueKind::Field { field, .. } if field.descriptor == "Z" => program
                    .resolve_field(&field.owner, &field.name, &field.descriptor)
                    .filter(|field| program.is_own(field.class))
                    .map_or(Evidence::Unknown, |field| {
                        Evidence::Depends(Fact::Field(field))
                    }),
                ValueKind::Call { kind, method, .. } => {
                    call_evidence(program, Some(member.class), *kind, method)
                }
                _ => Evidence::Unknown,
            };
            match evidence {
                Evidence::Canonical => (),
                Evidence::Depends(input) => {
                    inputs.insert(input);
                }
                Evidence::Unknown => {
                    known = false;
                    break;
                }
            }
        }
        let mut inputs: Vec<_> = inputs.into_iter().collect();
        inputs.sort_unstable();
        dependencies.push(Dependency {
            target,
            known,
            inputs,
        });
    }
    MethodSummary { dependencies }
}

#[cfg(test)]
#[path = "../../tests/unit/boolean_producers.rs"]
mod tests;
