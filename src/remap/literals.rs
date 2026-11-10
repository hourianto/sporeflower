use super::descriptor;
use crate::hash::HashSet;
use crate::{
    Result,
    bytecode::{Operand, decode},
    ir::{Literal, ValueKind},
    mappings::{MappingPlan, Target},
    naming::NamePlan,
    program::{MemberId, Program},
    semantics::Analysis,
};
use anyhow::{Context, bail, ensure};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Plan {
    pub methods: BTreeMap<usize, MethodPlan>,
    pub fields: BTreeMap<usize, Replacement>,
}

#[derive(Default)]
pub(super) struct MethodPlan {
    pub producers: BTreeMap<u32, Replacement>,
    pub groups: Vec<crate::semantics::ClassNameGroup>,
    pub uninitialized_stores: BTreeSet<u32>,
}

impl MethodPlan {
    pub fn rewrites(&self, pc: u32) -> bool {
        self.groups
            .iter()
            .find(|group| group.origins.contains(&pc))
            .is_none_or(|group| {
                group.decision == crate::semantics::ClassNameDecision::RewriteProducers
            })
    }

    pub fn require_adapter(&mut self, pc: u32) -> Result<()> {
        let group = self.groups.iter_mut().find(|group| group.origins.contains(&pc))
            .context("ClassName literal requires ldc widening; no scalar consumer adapter covers its origin")?;
        group.require_adapter();
        ensure!(
            group.decision != crate::semantics::ClassNameDecision::Reject,
            "ClassName literal at bytecode {pc} requires ldc widening; unsupported shared ClassName consumer includes a return, array store, or opaque input"
        );
        Ok(())
    }
}

pub(super) struct Replacement {
    pub text: String,
    pub original: u16,
}

/// Only fully allocated indices and selected boundary adapters reach emission.
#[derive(Default)]
pub(super) struct ResolvedPlan {
    pub methods: BTreeMap<usize, BTreeMap<u32, u16>>,
    pub fields: BTreeMap<usize, u16>,
    pub adapters: BTreeMap<usize, BTreeMap<u32, Vec<AdapterInput>>>,
    pub uninitialized_stores: BTreeSet<(usize, u32)>,
}

pub(super) enum AdapterInput {
    Argument {
        parameter: usize,
        names: Vec<String>,
    },
    Field {
        names: Vec<String>,
    },
}

impl Plan {
    pub fn finish(self, names: &NamePlan, mut resolved: ResolvedPlan) -> ResolvedPlan {
        use crate::semantics::{ClassNameDecision, ClassNameDestination};
        for (method, plan) in self.methods {
            resolved
                .uninitialized_stores
                .extend(plan.uninitialized_stores.iter().map(|&pc| (method, pc)));
            for group in plan.groups {
                if group.decision != ClassNameDecision::AdaptConsumers {
                    continue;
                }
                for input in group.inputs {
                    if !input.names.iter().any(|name| {
                        crate::semantics::class_names::relocated(name, names)
                            .is_some_and(|mapped| mapped != *name)
                    }) {
                        continue;
                    }
                    let (pc, input) = match input.destination {
                        ClassNameDestination::Argument { offset, parameter } => (
                            offset,
                            AdapterInput::Argument {
                                parameter,
                                names: input.names,
                            },
                        ),
                        ClassNameDestination::Field { offset } => {
                            (offset, AdapterInput::Field { names: input.names })
                        }
                        _ => unreachable!("a selected adapter has scalar destinations"),
                    };
                    resolved
                        .adapters
                        .entry(method)
                        .or_default()
                        .entry(pc)
                        .or_default()
                        .push(input);
                }
            }
        }
        resolved
    }
}

pub(super) fn plan(
    program: &Program<'_>,
    names: &NamePlan,
    mappings: &MappingPlan,
    owner: usize,
    warnings: &mut Vec<String>,
) -> Result<Plan> {
    let class = program.class(owner);
    let mut result = Plan::default();
    for (index, field) in class.fields.iter().enumerate() {
        let member = MemberId {
            class: owner,
            index,
        };
        if mappings
            .contract(Target::Field(member))
            .is_some_and(|contract| contract.class_name)
            && let Some(constant) = field.constant_value
            && let Ok(units) = class.constant_pool.string_utf16(constant)
            && let Some(text) = replacement(&units, names)?
        {
            result.fields.insert(
                index,
                Replacement {
                    text,
                    original: constant,
                },
            );
        }
    }
    let contracts = mappings
        .bindings()
        .values()
        .any(|contract| contract.class_name);
    for (index, method) in class.methods.iter().enumerate() {
        let Some(code) = &method.code else { continue };
        let instructions = decode(&code.bytes)?;
        let has_candidate = instructions.iter().any(|instruction| {
            matches!(instruction.opcode, 0x12 | 0x13)
                && match instruction.operand {
                    Operand::Constant(index) => class
                        .constant_pool
                        .string_utf16(index)
                        .ok()
                        .and_then(|units| replacement(&units, names).ok().flatten())
                        .is_some(),
                    _ => false,
                }
        });
        if !has_candidate {
            continue;
        }
        let reflection = instructions.iter().any(|instruction| {
            if !matches!(instruction.opcode, 0xb6..=0xb9) {
                return false;
            }
            let index = match instruction.operand {
                Operand::Constant(index) | Operand::InterfaceCall { index, .. } => index,
                _ => return false,
            };
            class.constant_pool.member_ref(index).is_ok_and(|member| {
                let method = crate::ir::Member {
                    owner: member.owner.into(),
                    name: member.name.into(),
                    descriptor: member.descriptor.into(),
                };
                crate::semantics::class_names::is_for_name(&method)
                    || program
                        .class_name_consumers()
                        .has_arguments(program, &method)
            })
        });
        if !contracts && !reflection {
            continue;
        }
        let member = MemberId {
            class: owner,
            index,
        };
        let ir = program.decompile_method(member, &[]).with_context(|| {
            format!(
                "analyze ClassName literals in {}.{}{}",
                class.name, method.name, method.descriptor
            )
        })?;
        let analysis = Analysis::analyze(&ir, program, mappings, member);
        let mut method_plan = MethodPlan::default();
        for diagnostic in &analysis.diagnostics {
            warnings.push(format!(
                "{}.{}{}: {diagnostic}",
                class.name, method.name, method.descriptor
            ));
        }
        for value in ir.values.ids() {
            let data = &ir.values[value];
            let ValueKind::Literal(Literal::String(units)) = &data.kind else {
                continue;
            };
            let Some(text) = replacement(units, names)? else {
                continue;
            };
            ensure!(
                !analysis.shared_class_name_literal(value)
                    || analysis
                        .class_name_groups()
                        .iter()
                        .any(|group| group.origins.contains(&data.pc)),
                "unsupported shared ClassName consumer in {}.{}{} at bytecode {}: no complete scalar adapter covers its return, array store, or opaque input; separate the ordinary and ClassName definitions",
                class.name,
                method.name,
                method.descriptor,
                data.pc
            );
            if !analysis.class_name_literal(value) {
                continue;
            }
            let instruction = instructions
                .iter()
                .find(|instruction| instruction.pc == data.pc)
                .context("ClassName origin has no original instruction")?;
            ensure!(
                matches!(instruction.opcode, 0x12 | 0x13),
                "ClassName origin is not an original ldc"
            );
            let Operand::Constant(original) = instruction.operand else {
                bail!("ClassName ldc has no constant index")
            };
            method_plan
                .producers
                .insert(data.pc, Replacement { text, original });
        }
        for group in analysis.into_class_name_groups() {
            if !group
                .inputs
                .iter()
                .flat_map(|input| &input.names)
                .any(|name| {
                    crate::semantics::class_names::relocated(name, names)
                        .is_some_and(|mapped| mapped != *name)
                })
            {
                continue;
            }
            ensure!(
                group.decision != crate::semantics::ClassNameDecision::Reject,
                "unsupported shared ClassName consumer in {}.{}{}: no complete scalar adapter covers its return, array store, or opaque input; separate the ordinary and ClassName definitions",
                class.name,
                method.name,
                method.descriptor
            );
            for input in &group.inputs {
                if let crate::semantics::ClassNameDestination::Field { offset } = input.destination
                    && method.name == "<init>"
                    && uninitialized_this_store(&ir, offset)
                {
                    method_plan.uninitialized_stores.insert(offset);
                }
            }
            method_plan.groups.push(group);
        }
        if !method_plan.producers.is_empty() || !method_plan.groups.is_empty() {
            result.methods.insert(index, method_plan);
        }
    }
    Ok(result)
}

fn uninitialized_this_store(ir: &crate::ir::MethodIr, pc: u32) -> bool {
    use crate::ir::{Statement, ValueId};
    fn this(ir: &crate::ir::MethodIr, value: ValueId, seen: &mut HashSet<ValueId>) -> bool {
        if !seen.insert(value) {
            return true;
        }
        match &ir.values[value].kind {
            ValueKind::Parameter { index: None, .. } => true,
            ValueKind::Alias(value)
            | ValueKind::Cast { value, .. }
            | ValueKind::InlineAssign { value, .. } => this(ir, *value, seen),
            ValueKind::Phi { inputs, .. } => {
                !inputs.is_empty() && inputs.iter().all(|input| this(ir, input.value, seen))
            }
            _ => false,
        }
    }
    let mut pending = vec![(ir.entry, false)];
    let mut seen = HashSet::default();
    while let Some((block, mut initialized)) = pending.pop() {
        if !seen.insert((block, initialized)) {
            continue;
        }
        let block = &ir.blocks[block];
        // Exception handlers may be entered before any statement completes.
        pending.extend(
            block
                .handlers
                .iter()
                .map(|handler| (handler.target, initialized)),
        );
        for statement in &block.statements {
            match statement {
                Statement::ConstructorCall { receiver, .. }
                    if this(ir, *receiver, &mut HashSet::default()) =>
                {
                    initialized = true
                }
                Statement::PutField {
                    pc: Some(offset),
                    receiver: Some(receiver),
                    ..
                } if *offset == pc
                    && !initialized
                    && this(ir, *receiver, &mut HashSet::default()) =>
                {
                    return true;
                }
                _ => {}
            }
        }
        pending.extend(
            block
                .terminator
                .successors()
                .into_iter()
                .map(|target| (target, initialized)),
        );
    }
    false
}

fn replacement(units: &[u16], names: &NamePlan) -> Result<Option<String>> {
    let Ok(original) = String::from_utf16(units) else {
        return Ok(None);
    };
    if original.contains('/') {
        return Ok(None);
    }
    let internal = original.replace('.', "/");
    let mapped = if internal.starts_with('[') {
        if crate::classfile::JvmType::parse(&internal).is_err() {
            return Ok(None);
        }
        descriptor(&internal, names)?.replace('/', ".")
    } else {
        names.class_name_for(&internal).replace('/', ".")
    };
    Ok((mapped != original).then_some(mapped))
}
