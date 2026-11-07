//! Original-identity planning for allocation and constant-read initialization.
//! Only methods whose pending NEW crosses a possible source boundary need the
//! full SSA analysis. The plan retains sites and owners, never whole method IR.
use crate::bytecode::{Instruction, Operand};
use crate::classfile::{ClassFile, Code, MethodDescriptor};
use crate::ir::InitializationSite;
use crate::program::{ClassId, MemberId, Program};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Debug, Default)]
pub struct InitializationStatistics {
    pub candidate_methods: usize,
    pub moved_boundaries: usize,
    pub trigger_sites: usize,
    pub trigger_classes: usize,
    pub external_sites: usize,
}

pub struct InitializationPlan {
    sites: HashMap<MemberId, Vec<InitializationSite>>,
    external: HashMap<MemberId, Vec<InitializationSite>>,
    classes: BTreeSet<ClassId>,
    non_initializer_finals: BTreeSet<MemberId>,
    written_finals: BTreeSet<MemberId>,
    pub statistics: InitializationStatistics,
}

impl InitializationPlan {
    /// Original writes outside the declaring initializer cannot be assignments
    /// to a Java final. Keep this fact shared across independently emitted classes.
    pub(crate) fn permits_final(&self, field: MemberId) -> bool {
        !self.non_initializer_finals.contains(&field)
    }
    pub(crate) fn writes_final(&self, field: MemberId) -> bool {
        self.written_finals.contains(&field)
    }
    pub fn sites(&self, member: MemberId) -> &[InitializationSite] {
        self.sites.get(&member).map_or(&[], Vec::as_slice)
    }
    pub fn unresolved(&self, member: MemberId) -> &[InitializationSite] {
        self.external.get(&member).map_or(&[], Vec::as_slice)
    }
    pub fn classes(&self) -> impl Iterator<Item = ClassId> + '_ {
        self.classes.iter().copied()
    }
    pub fn external_sites(&self) -> impl Iterator<Item = (MemberId, &InitializationSite)> {
        self.external
            .iter()
            .flat_map(|(&member, sites)| sites.iter().map(move |site| (member, site)))
    }
    pub(crate) fn build(program: &Program<'_>) -> Self {
        let mut plan = Self {
            sites: HashMap::new(),
            external: HashMap::new(),
            classes: BTreeSet::new(),
            non_initializer_finals: BTreeSet::new(),
            written_finals: BTreeSet::new(),
            statistics: InitializationStatistics::default(),
        };
        for owner in 0..program.own_count() {
            let class = program.class(owner);
            for (index, method) in class.methods.iter().enumerate() {
                let member = MemberId {
                    class: owner,
                    index,
                };
                let mut constant_reads = HashMap::new();
                let allocation_candidate = if let Some(code) = &method.code
                    && let Ok(instructions) = crate::bytecode::decode(&code.bytes)
                {
                    for instruction in &instructions {
                        if !matches!(instruction.opcode, 0xb2 | 0xb3 | 0xb5) {
                            continue;
                        }
                        let Operand::Constant(reference_index) = instruction.operand else {
                            continue;
                        };
                        let Ok(reference) = class.constant_pool.member_ref(reference_index) else {
                            continue;
                        };
                        let Some(field) = program.resolve_field(
                            reference.owner,
                            reference.name,
                            reference.descriptor,
                        ) else {
                            continue;
                        };
                        let declaration = program.field(field);
                        if instruction.opcode == 0xb2 {
                            // javac may inline a constant variable, but an original
                            // GETSTATIC still initializes the declaring class.
                            // Resolution/access failures precede initialization.
                            if declaration.access_flags & 0x0018 == 0x0018
                                && declaration.constant_value.is_some()
                                && program.is_own(field.class)
                                && program.member_accessible_from(
                                    declaration.access_flags,
                                    field.class,
                                    owner,
                                )
                            {
                                constant_reads.insert(
                                    instruction.pc,
                                    program.class(field.class).name.clone(),
                                );
                            }
                            continue;
                        }
                        // A forbidden final store throws IllegalAccessError;
                        // it neither initializes the field nor makes it mutable.
                        if declaration.access_flags & 0x0010 == 0
                            || !program.field_write_permitted(field, member)
                        {
                            continue;
                        }
                        plan.written_finals.insert(field);
                        let initializer = if declaration.access_flags & 0x0008 != 0 {
                            "<clinit>"
                        } else {
                            "<init>"
                        };
                        if method.name != initializer {
                            plan.non_initializer_finals.insert(field);
                        }
                    }
                    candidate(program, class, code, &instructions).unwrap_or(false)
                } else {
                    false
                };
                if !allocation_candidate && constant_reads.is_empty() {
                    continue;
                }
                plan.statistics.candidate_methods += 1;
                let mut sites = Vec::new();
                if allocation_candidate && let Ok(ir) = program.decompile_method(member, &[]) {
                    plan.statistics.moved_boundaries += ir.initialization_sites.len();
                    sites = ir.initialization_sites.clone();
                    // A missing or inaccessible constructor is lowered to a caller-local linkage
                    // throw. Its NEW still initializes the class before evaluating
                    // constructor arguments, even if ordinary NEW never moved.
                    for site in failed_constructor_sites(program, owner, &ir) {
                        if !sites
                            .iter()
                            .any(|other| other.pc == site.pc && other.class == site.class)
                        {
                            sites.push(site);
                        }
                    }
                }
                if !constant_reads.is_empty()
                    && let Some(code) = &method.code
                    && let Ok(prepared) = crate::cfg::PreparedCode::new(code)
                {
                    for (index, instruction) in prepared.instructions.iter().enumerate() {
                        if instruction.opcode == 0xb2
                            && let Some(class) = constant_reads.get(&prepared.original_pc(index))
                        {
                            sites.push(InitializationSite {
                                class: class.clone(),
                                pc: instruction.pc,
                                block: prepared.graph.instruction_blocks[index],
                                statement_start: 0,
                            });
                        }
                    }
                }
                for site in sites {
                    if site.class == class.name
                        || (program
                            .id(&site.class)
                            .is_some_and(|target| program.class(target).access_flags & 0x0200 == 0)
                            && program
                                .ancestors(owner)
                                .iter()
                                .any(|&ancestor| program.class(ancestor).name == site.class))
                    {
                        continue;
                    }
                    if quiet_initialization(program, &site.class, &mut BTreeSet::new()) {
                        continue;
                    }
                    if let Some(target) = program
                        .id(&site.class)
                        .filter(|&target| program.is_own(target))
                    {
                        plan.classes.insert(target);
                        plan.statistics.trigger_sites += 1;
                        plan.sites.entry(member).or_default().push(site);
                    } else {
                        plan.statistics.external_sites += 1;
                        plan.external.entry(member).or_default().push(site);
                    }
                }
            }
        }
        plan.statistics.trigger_classes = plan.classes.len();
        plan
    }
}

/// CLDC's core value/buffer/exception classes expose no user-observable class
/// initialization hook. This deliberately excludes mutable utility state (for
/// example Random) and all vendor/optional SDK classes.
fn core_initialization_is_quiet(name: &str) -> bool {
    matches!(
        name,
        "java/lang/Object"
            | "java/lang/String"
            | "java/lang/StringBuffer"
            | "java/lang/StringBuilder"
            | "java/lang/Throwable"
            | "java/lang/Exception"
            | "java/lang/RuntimeException"
            | "java/lang/Error"
            | "java/lang/IllegalArgumentException"
            | "java/lang/IllegalStateException"
            | "java/lang/NullPointerException"
            | "java/lang/ArithmeticException"
            | "java/lang/IndexOutOfBoundsException"
            | "java/lang/ArrayIndexOutOfBoundsException"
            | "java/lang/NegativeArraySizeException"
            | "java/lang/ArrayStoreException"
            | "java/lang/ClassCastException"
            | "java/lang/NumberFormatException"
            | "java/lang/ExceptionInInitializerError"
            | "java/lang/OutOfMemoryError"
    )
}

fn quiet_initialization(
    program: &Program<'_>,
    name: &str,
    visiting: &mut BTreeSet<String>,
) -> bool {
    if core_initialization_is_quiet(name) {
        return true;
    }
    let Some(id) = program.id(name).filter(|&id| program.is_own(id)) else {
        return false;
    };
    if !visiting.insert(name.to_owned()) {
        return false;
    }
    let class = program.class(id);
    if class.methods.iter().any(|method| method.name == "<clinit>") {
        return false;
    }
    class
        .super_name
        .as_deref()
        .is_none_or(|parent| quiet_initialization(program, parent, visiting))
}

/// A Java field initializer runs after superclass initialization, whereas a
/// ConstantValue is installed before it. Moving a mutable constant's value is
/// safe only when no superclass initialization code can observe that interval.
pub(crate) fn superclass_initialization_is_quiet(program: &Program<'_>, owner: ClassId) -> bool {
    program
        .class(owner)
        .super_name
        .as_deref()
        .is_none_or(|parent| quiet_initialization(program, parent, &mut BTreeSet::new()))
}

fn candidate(
    program: &Program<'_>,
    class: &ClassFile,
    code: &Code,
    instructions: &[Instruction],
) -> crate::Result<bool> {
    // The raw byte check is only a fast rejection; instructions are already decoded.
    if !code.bytes.contains(&0xbb) {
        return Ok(false);
    }
    let boundaries: BTreeSet<_> = code
        .exception_table
        .iter()
        .flat_map(|handler| [handler.start_pc, handler.end_pc])
        .collect();
    let mut pending = Vec::new();
    for instruction in instructions {
        if !pending.is_empty() && boundaries.contains(&instruction.pc) {
            return Ok(true);
        }
        if instruction.opcode == 0xbb {
            if !pending.is_empty() {
                return Ok(true);
            }
            let Operand::Constant(index) = instruction.operand else {
                continue;
            };
            pending.push(class.constant_pool.class_name(index)?);
        } else if !pending.is_empty() {
            if matches!(instruction.opcode, 0x36..=0x3a | 0x4f..=0x58 | 0x84 | 0x99..=0xb1 | 0xb3 | 0xb5 | 0xbf | 0xc2 | 0xc3 | 0xc6 | 0xc7)
            {
                return Ok(true);
            }
            if matches!(instruction.opcode, 0xb6..=0xb9) {
                let index = match instruction.operand {
                    Operand::Constant(index) | Operand::InterfaceCall { index, .. } => index,
                    _ => continue,
                };
                let target = class.constant_pool.member_ref(index)?;
                if target.name == "<init>" {
                    if program
                        .id(target.owner)
                        .filter(|&id| program.is_own(id))
                        .is_some_and(|id| {
                            failed_constructor(
                                program,
                                id,
                                target.descriptor,
                                program.id(&class.name).unwrap(),
                            )
                        })
                    {
                        return Ok(true);
                    }
                    if pending.last().copied() != Some(target.owner) {
                        return Ok(true);
                    }
                    pending.pop();
                } else if MethodDescriptor::parse(target.descriptor)?.return_type
                    == crate::classfile::JvmType::Void
                {
                    return Ok(true);
                }
            }
        }
    }
    Ok(!pending.is_empty())
}

fn failed_constructor_sites(
    program: &Program<'_>,
    owner: ClassId,
    ir: &crate::ir::MethodIr,
) -> Vec<InitializationSite> {
    use crate::ir::{ValueKind, ValueUse};
    let mut pending = Vec::new();
    for block in &ir.blocks {
        for statement in &block.statements {
            crate::ir::scheduled::for_each_statement_root(statement, |usage| pending.push(usage));
        }
        pending.extend(
            block
                .terminator
                .operands()
                .into_iter()
                .map(ValueUse::Reference),
        );
    }
    let allocations: HashMap<_, _> = ir
        .values
        .iter()
        .filter_map(|value| {
            if let ValueKind::Allocation { class } = &value.kind {
                Some(((value.pc, class.as_str()), value.block))
            } else {
                None
            }
        })
        .collect();
    let mut seen = BTreeSet::new();
    let mut sites = Vec::new();
    while let Some(usage) = pending.pop() {
        let id = usage.value();
        if usage.local(ir).is_some() || !seen.insert(id) {
            continue;
        }
        let value = &ir.values[id];
        if let ValueKind::New {
            class,
            descriptor,
            allocation_pc,
            ..
        } = &value.kind
            && program
                .id(class)
                .filter(|&id| program.is_own(id))
                .is_some_and(|id| failed_constructor(program, id, descriptor, owner))
            && let Some(&block) = allocations.get(&(*allocation_pc, class.as_str()))
            && !sites
                .iter()
                .any(|site: &InitializationSite| site.pc == *allocation_pc && site.class == *class)
        {
            sites.push(InitializationSite {
                class: class.clone(),
                pc: *allocation_pc,
                block,
                statement_start: 0,
            });
        }
        usage.for_each_child(ir, |operand| pending.push(operand));
    }
    sites
}

fn failed_constructor(
    program: &Program<'_>,
    target: ClassId,
    descriptor: &str,
    caller: ClassId,
) -> bool {
    program
        .declared_method(target, "<init>", descriptor)
        .is_none_or(|constructor| {
            !program.member_accessible_from(
                program.method(constructor).access_flags,
                target,
                caller,
            )
        })
}
