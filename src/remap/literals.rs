use crate::hash::HashSet;
use crate::{
    Result,
    bytecode::{Operand, decode},
    ir::{Literal, MethodIr, Statement, ValueKind},
    mappings::{MappingPlan, Target},
    naming::NamePlan,
    program::{MemberId, Program},
    semantics::Analysis,
};
use anyhow::{Context, ensure};
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

#[derive(Clone)]
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
                    if !input
                        .names
                        .iter()
                        .any(|name| replacement(name, names).is_some())
                    {
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
            && let Some(text) = literal_replacement(&units, names)
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
        let loads: BTreeMap<_, _> = instructions
            .iter()
            .filter_map(|instruction| {
                if !matches!(instruction.opcode, 0x12 | 0x13) {
                    return None;
                }
                let Operand::Constant(original) = instruction.operand else {
                    return None;
                };
                let units = class.constant_pool.string_utf16(original).ok()?;
                let text = literal_replacement(&units, names)?;
                Some((instruction.pc, Replacement { text, original }))
            })
            .collect();
        if loads.is_empty() {
            continue;
        }
        let reflection = instructions.iter().any(|instruction| {
            if !matches!(instruction.opcode, 0xb6..=0xb9) {
                return false;
            }
            instruction.member_ref(class).is_some_and(|member| {
                let method = member.into();
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
        let prepared = crate::cfg::PreparedCode::from_decoded(code, instructions)?;
        let (mut ir, _) = program
            .lift_prepared(member, &prepared, &[], true)
            .with_context(|| {
                format!(
                    "analyze ClassName literals in {}.{}{}",
                    class.name, method.name, method.descriptor
                )
            })?;
        if let Some(origins) = prepared.subroutine_origins() {
            bind_subroutine_origins(&mut ir, origins);
        }
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
            let ValueKind::Literal(Literal::String(_)) = &data.kind else {
                continue;
            };
            let Some(replacement) = loads.get(&data.pc) else {
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
            method_plan.producers.insert(data.pc, replacement.clone());
        }
        for group in analysis.into_class_name_groups() {
            if !group
                .inputs
                .iter()
                .flat_map(|input| &input.names)
                .any(|name| replacement(name, names).is_some())
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

/// Native patches and offset-scoped contracts address the single instruction
/// in the class file. Clones retain separate control flow and local bindings,
/// but every use of one original ldc must participate in one relocation decision.
fn bind_subroutine_origins(ir: &mut MethodIr, origins: &[u32]) {
    let mut literals = BTreeMap::new();
    for id in ir.values.ids() {
        let value = &mut ir.values[id];
        value.pc = origins[value.pc as usize];
        if matches!(value.kind, ValueKind::Literal(Literal::String(_))) {
            let literal = *literals.entry(value.pc).or_insert_with(|| {
                let mut literal = ir.values[id].clone();
                literal.local = None;
                ir.values.push(literal)
            });
            ir.values[id].kind = ValueKind::Alias(literal);
        }
    }
    for statement in ir.blocks.iter_mut().flat_map(|block| &mut block.statements) {
        if let Statement::PutField { pc: Some(pc), .. } | Statement::ConstructorCall { pc, .. } =
            statement
        {
            *pc = origins[*pc as usize];
        }
    }
}

fn uninitialized_this_store(ir: &crate::ir::MethodIr, pc: u32) -> bool {
    let receiver_is_this = |value| {
        ir.traces_to_this(value, |kind| match kind {
            ValueKind::Cast { value, .. } | ValueKind::InlineAssign { value, .. } => Some(*value),
            _ => None,
        })
    };
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
                Statement::ConstructorCall { receiver, .. } if receiver_is_this(*receiver) => {
                    initialized = true
                }
                Statement::PutField {
                    pc: Some(offset),
                    receiver: Some(receiver),
                    ..
                } if *offset == pc && !initialized && receiver_is_this(*receiver) => {
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

fn literal_replacement(units: &[u16], names: &NamePlan) -> Option<String> {
    replacement(&String::from_utf16(units).ok()?, names)
}

pub(super) fn replacement(original: &str, names: &NamePlan) -> Option<String> {
    crate::semantics::class_names::relocated(original, names).filter(|mapped| mapped != original)
}
