//! Original-identity planning for allocation and constant-read initialization.
//! Only methods whose pending NEW crosses a possible source boundary need the
//! full SSA analysis. The plan retains sites and owners, never whole method IR.
use crate::bytecode::{Instruction, Operand};
use crate::classfile::{Code, FINAL, MethodDescriptor, STATIC};
use crate::hash::HashMap;
use crate::ir::InitializationSite;
use crate::program::{ClassId, MemberId, MethodScans, Program};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Clone, Debug, Default)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct InitializationStatistics {
    pub candidate_methods: usize,
    pub moved_boundaries: usize,
    pub trigger_sites: usize,
    pub trigger_classes: usize,
    pub external_sites: usize,
}

/// Facts read directly from bytecode are identical for both source modes.
#[derive(Default)]
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
struct BytecodeFacts {
    allocations: Vec<MemberId>,
    constants: Vec<(MemberId, InitializationSite, ClassId)>,
    non_initializer_finals: BTreeSet<MemberId>,
    written_finals: BTreeSet<MemberId>,
    candidate_methods: usize,
}

#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub struct InitializationPlan {
    original: Arc<BytecodeFacts>,
    sites: HashMap<MemberId, Vec<InitializationSite>>,
    external: HashMap<MemberId, Vec<InitializationSite>>,
    classes: BTreeSet<ClassId>,
    pub statistics: InitializationStatistics,
}

impl InitializationPlan {
    pub(crate) fn permits_final(&self, field: MemberId) -> bool {
        !self.original.non_initializer_finals.contains(&field)
    }
    pub(crate) fn writes_final(&self, field: MemberId) -> bool {
        self.original.written_finals.contains(&field)
    }
    pub(crate) fn needs_helper(&self, class: ClassId) -> bool {
        self.classes.contains(&class)
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
    pub(crate) fn build(program: &Program<'_>, scans: MethodScans) -> Self {
        let mut facts = BytecodeFacts::default();
        for (member, scan) in scans.map(program.owned_methods(), |member| {
            (member, scan_method(program, member))
        }) {
            if scan.allocation_candidate {
                facts.allocations.push(member);
            }
            facts.candidate_methods += usize::from(scan.candidate);
            facts.constants.extend(
                scan.constants
                    .into_iter()
                    .map(|(site, target)| (member, site, target)),
            );
            facts.written_finals.extend(scan.written_finals);
            facts
                .non_initializer_finals
                .extend(scan.non_initializer_finals);
        }
        Self::place_allocations(program, Arc::new(facts), false, scans)
    }

    pub(crate) fn inspection(&self, program: &Program<'_>, scans: MethodScans) -> Self {
        Self::place_allocations(program, Arc::clone(&self.original), true, scans)
    }

    fn place_allocations(
        program: &Program<'_>,
        original: Arc<BytecodeFacts>,
        preserve: bool,
        scans: MethodScans,
    ) -> Self {
        let mut plan = Self {
            sites: HashMap::default(),
            external: HashMap::default(),
            classes: BTreeSet::new(),
            statistics: InitializationStatistics {
                candidate_methods: original.candidate_methods,
                ..Default::default()
            },
            original: Arc::clone(&original),
        };
        for (member, (moved, sites)) in scans.map(original.allocations.iter().copied(), |member| {
            (member, allocation_sites(program, member, preserve))
        }) {
            plan.statistics.moved_boundaries += moved;
            for site in sites {
                let target = program.id(&site.class);
                if needs_trigger(program, member.class, target, &site.class) {
                    plan.record(member, site, target.filter(|&id| program.is_own(id)));
                }
            }
        }
        for (member, site, target) in &original.constants {
            plan.record(*member, site.clone(), Some(*target));
        }
        plan.statistics.trigger_classes = plan.classes.len();
        plan
    }

    fn record(&mut self, member: MemberId, site: InitializationSite, target: Option<ClassId>) {
        if let Some(target) = target {
            self.classes.insert(target);
            self.statistics.trigger_sites += 1;
            self.sites.entry(member).or_default().push(site);
        } else {
            self.statistics.external_sites += 1;
            self.external.entry(member).or_default().push(site);
        }
    }
}

#[derive(Default)]
struct MethodScan {
    written_finals: Vec<MemberId>,
    non_initializer_finals: Vec<MemberId>,
    candidate: bool,
    allocation_candidate: bool,
    constants: Vec<(InitializationSite, ClassId)>,
}

fn scan_method(program: &Program<'_>, member: MemberId) -> MethodScan {
    let owner = member.class;
    let class = program.class(owner);
    let method = program.method(member);
    let mut scan = MethodScan::default();
    let Some(code) = &method.code else {
        return scan;
    };
    // Operand bytes can produce false positives here, never false negatives.
    if !code
        .bytes
        .iter()
        .any(|&byte| matches!(byte, 0xb2 | 0xb3 | 0xb5 | 0xbb))
    {
        return scan;
    }
    let Ok(instructions) = crate::bytecode::decode(&code.bytes) else {
        return scan;
    };
    let mut constant_reads = HashMap::default();
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
        let Some(field) =
            program.resolve_field(reference.owner, reference.name, reference.descriptor)
        else {
            continue;
        };
        let declaration = program.field(field);
        if instruction.opcode == 0xb2 {
            // javac may inline a constant variable, but an original
            // GETSTATIC still initializes the declaring class.
            // Resolution/access failures precede initialization.
            if declaration.access_flags & (STATIC | FINAL) == (STATIC | FINAL)
                && declaration.constant_value.is_some()
                && program.is_own(field.class)
                && program.member_accessible_from(declaration.access_flags, field.class, owner)
            {
                constant_reads.insert(instruction.pc, field.class);
            }
            continue;
        }
        // A forbidden final store throws IllegalAccessError;
        // it neither initializes the field nor makes it mutable.
        if declaration.access_flags & FINAL == 0 || !program.field_write_permitted(field, member) {
            continue;
        }
        scan.written_finals.push(field);
        let initializer = if declaration.access_flags & STATIC != 0 {
            "<clinit>"
        } else {
            "<init>"
        };
        if method.name != initializer {
            scan.non_initializer_finals.push(field);
        }
    }
    scan.allocation_candidate = candidate(program, owner, code, &instructions).unwrap_or(false);
    scan.candidate = scan.allocation_candidate || !constant_reads.is_empty();
    if !constant_reads.is_empty()
        && let Ok(prepared) = crate::cfg::PreparedCode::from_decoded(code, instructions)
    {
        for (index, instruction) in prepared.instructions.iter().enumerate() {
            if instruction.opcode == 0xb2
                && let Some(&target) = constant_reads.get(&prepared.original_pc(index))
                && needs_trigger(program, owner, Some(target), &program.class(target).name)
            {
                scan.constants.push((
                    InitializationSite {
                        class: program.class(target).name.clone(),
                        pc: instruction.pc,
                        block: prepared.graph.instruction_blocks[index],
                        statement_start: 0,
                    },
                    target,
                ));
            }
        }
    }
    scan
}

fn allocation_sites(
    program: &Program<'_>,
    member: MemberId,
    preserve: bool,
) -> (usize, Vec<InitializationSite>) {
    let Ok((ir, _)) = program.decompile_method(member, &[], preserve) else {
        return (0, Vec::new());
    };
    let moved = ir.initialization_sites.len();
    // Missing or inaccessible constructors use a caller-local linkage throw.
    // Their original NEW still initializes the class before the arguments.
    let failed = failed_constructor_sites(program, member.class, &ir);
    let mut sites = ir.initialization_sites;
    for site in failed {
        if !sites
            .iter()
            .any(|other| other.pc == site.pc && other.class == site.class)
        {
            sites.push(site);
        }
    }
    (moved, sites)
}

fn needs_trigger(
    program: &Program<'_>,
    owner: ClassId,
    target: Option<ClassId>,
    name: &str,
) -> bool {
    target != Some(owner)
        && !target.is_some_and(|id| {
            program.class(id).access_flags & crate::classfile::INTERFACE == 0
                && program.ancestors(owner).contains(&id)
        })
        && !quiet_initialization(program, name)
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

pub(crate) fn quiet_initialization(program: &Program<'_>, name: &str) -> bool {
    // Program construction has already checked the superclass chain for cycles.
    let mut current = Some(name);
    while let Some(name) = current {
        let Some(id) = program.id(name).filter(|&id| program.is_own(id)) else {
            return core_initialization_is_quiet(name) && program.can_use_platform_contract(name);
        };
        let class = program.class(id);
        if class.methods.iter().any(|method| method.name == "<clinit>") {
            return false;
        }
        current = class.super_name.as_deref();
    }
    true
}

/// A Java field initializer runs after superclass initialization, whereas a
/// ConstantValue is installed before it. Moving a mutable constant's value is
/// safe only when no superclass initialization code can observe that interval.
pub(crate) fn superclass_initialization_is_quiet(program: &Program<'_>, owner: ClassId) -> bool {
    program
        .class(owner)
        .super_name
        .as_deref()
        .is_none_or(|parent| quiet_initialization(program, parent))
}

fn candidate(
    program: &Program<'_>,
    owner: ClassId,
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
    let pool = &program.class(owner).constant_pool;
    let mut pending = None;
    for instruction in instructions {
        if pending.is_some() && boundaries.contains(&instruction.pc) {
            return Ok(true);
        }
        if instruction.opcode == 0xbb {
            if pending.is_some() {
                return Ok(true);
            }
            pending = Some(pool.class_name(instruction.constant_index()?)?);
        } else if let Some(allocated) = pending {
            if matches!(instruction.opcode, 0x36..=0x3a | 0x4f..=0x58 | 0x84 | 0x99..=0xb1 | 0xb3 | 0xb5 | 0xbf | 0xc2 | 0xc3 | 0xc6 | 0xc7)
            {
                return Ok(true);
            }
            if matches!(instruction.opcode, 0xb6..=0xb9) {
                let target = pool.member_ref(instruction.constant_index()?)?;
                if target.name == "<init>" {
                    if allocated != target.owner
                        || failed_constructor(program, target.owner, target.descriptor, owner)
                    {
                        return Ok(true);
                    }
                    pending = None;
                } else if MethodDescriptor::parse(target.descriptor)?.return_type
                    == crate::classfile::JvmType::Void
                {
                    return Ok(true);
                }
            }
        }
    }
    Ok(pending.is_some())
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
            && failed_constructor(program, class, descriptor, owner)
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
    target: &str,
    descriptor: &str,
    caller: ClassId,
) -> bool {
    let Some(target) = program.id(target).filter(|&id| program.is_own(id)) else {
        return false;
    };
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

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "../tests/unit/initialization_parallel.rs"]
mod parallel_tests;
