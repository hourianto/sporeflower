//! Compiler-owned enclosing instances of named member classes.
use super::context::EmissionContext;
use crate::{
    bytecode::{Instruction, Operand, decode},
    classfile::{
        ClassFile, Constant, FINAL, Method, MethodDescriptor, PROTECTED, PUBLIC, STATIC, VOLATILE,
    },
    hash::HashSet,
    ir::{MethodIr, Statement},
    naming::NamePlan,
    program::{ClassId, MemberId, Program},
};
use std::cell::OnceCell;

pub(super) struct Capture {
    pub field: MemberId,
    pub guards: Vec<(MemberId, u32)>,
}

/// Index constant-pool class references once, and only for a project with a
/// candidate enclosing-instance field. A field/method reference always names
/// its owner through one of these entries.
#[derive(Default)]
pub(super) struct Users(OnceCell<Vec<Vec<ClassId>>>);

impl Users {
    fn of(&self, program: &Program<'_>, owner: ClassId) -> &[ClassId] {
        &self.0.get_or_init(|| {
            let mut users = vec![Vec::new(); program.own_count()];
            for (id, class) in program.classes()[..program.own_count()].iter().enumerate() {
                for cp in &class.constant_pool.entries {
                    if let Constant::Class(name) = cp
                        && let Ok(name) = class.constant_pool.utf8(*name)
                        && let Some(referenced) =
                            program.id(name).filter(|&target| program.is_own(target))
                        && users[referenced].last() != Some(&id)
                    {
                        users[referenced].push(id);
                    }
                }
            }
            users
        })[owner]
    }
}

pub(super) fn capture(
    program: &Program<'_>,
    names: &NamePlan,
    owner: ClassId,
    enclosing: ClassId,
    users: &Users,
) -> Option<Capture> {
    let class = program.class(owner);
    if class.super_name.as_deref() != Some("java/lang/Object")
        || program.descendants(owner).len() > 1
        || program
            .id("java/lang/Object")
            .is_some_and(|id| program.is_own(id))
    {
        return None;
    }
    let field = program.declared_field(
        owner,
        "this$0",
        &format!("L{};", program.class(enclosing).name),
    )?;
    let capture = program.field(field);
    if names.is_mapped_field(field)
        || !capture.synthetic
        || capture.access_flags & (PUBLIC | PROTECTED | FINAL | STATIC | VOLATILE) != FINAL
        || !class.methods.iter().any(|m| m.name == "<init>")
        || class.fields.iter().enumerate().any(|(index, f)| {
            let id = MemberId {
                class: owner,
                index,
            };
            let init = program.initialization_plan();
            if f.access_flags & STATIC != 0 {
                return f.access_flags & FINAL == 0
                    || f.constant_value.is_none()
                    || !init.permits_final(id);
            }
            // A blank primitive final retaining its VM default would require
            // a static source helper to avoid becoming a compile-time constant.
            f.access_flags & FINAL != 0
                && !f.descriptor.starts_with(['L', '['])
                && init.permits_final(id)
                && !init.writes_final(id)
        })
    {
        return None;
    }
    for (index, method) in class.methods.iter().enumerate() {
        let id = MemberId {
            class: owner,
            index,
        };
        if method.name == "<init>"
            && names
                .parameter_name(id, 0)
                .is_some_and(|name| !name.is_empty())
        {
            return None;
        }
        check_member_body(program, field, method)?;
    }
    let mut guards = Vec::new();
    for &id in users.of(program, owner) {
        check_caller(program, id, field, enclosing, &mut guards)?;
    }
    Some(Capture { field, guards })
}

fn check_member_body(program: &Program<'_>, field: MemberId, method: &Method) -> Option<()> {
    let owner = field.class;
    let class = program.class(owner);
    let capture = program.field(field);
    let constructor = method.name == "<init>";
    let code = method.code.as_ref()?;
    if method.access_flags & STATIC != 0 || !code.exception_table.is_empty() {
        return None;
    }
    let ops = decode(&code.bytes).ok()?;
    let body = if constructor {
        if !method
            .descriptor
            .strip_prefix('(')?
            .starts_with(&capture.descriptor)
        {
            return None;
        }
        // javac has used both positions for this$0, depending on its target.
        let store = |ops: &[Instruction]| {
            matches!(ops, [this, outer, put]
                if aload(this, 0) && aload(outer, 1) && put.opcode == 0xb5
                    && references(class, put, &class.name, &capture.name, &capture.descriptor))
        };
        let object = |ops: &[Instruction]| {
            matches!(ops, [this, call]
                if aload(this, 0) && call.opcode == 0xb7 && references(class, call, "java/lang/Object", "<init>", "()V"))
        };
        let head = ops.get(..5)?;
        if !(store(&head[..3]) && object(&head[3..]) || object(&head[..2]) && store(&head[2..])) {
            return None;
        }
        &ops[5..]
    } else {
        &ops[..]
    };
    // Straight-line field initializers and accessors need no static Java
    // support declarations (illegal in non-static Java 1.3 member classes).
    for (offset, op) in body.iter().enumerate() {
        if constructor && aload(op, 1) {
            return None;
        }
        match op.opcode {
            0x12..=0x14 => match class.constant_pool.get(op.operand.constant_index()?).ok()? {
                Constant::Integer(_)
                | Constant::Float(_)
                | Constant::Long(_)
                | Constant::Double(_)
                | Constant::String(_) => {}
                _ => return None,
            },
            0x00..=0x11 | 0x15..=0x19 | 0x57..=0x83 | 0x85..=0x93 | 0xac..=0xb1 | 0xb4 | 0xb5 => {}
            _ => return None,
        }
        if matches!(op.opcode, 0xb4 | 0xb5) {
            let member = op.member_ref(class)?;
            let id = program.resolve_field(member.owner, member.name, member.descriptor)?;
            let flags = program.field(id).access_flags;
            if flags & STATIC != 0
                || !program.member_accessible_from(flags, id.class, owner)
                || op.opcode == 0xb5 && flags & FINAL != 0 && !(id.class == owner && constructor)
                || id == field
                    && (op.opcode != 0xb4
                        || !body[..offset].last().is_some_and(|prev| aload(prev, 0)))
            {
                return None;
            }
        }
    }
    Some(())
}

fn check_caller(
    program: &Program<'_>,
    id: ClassId,
    field: MemberId,
    enclosing: ClassId,
    guards: &mut Vec<(MemberId, u32)>,
) -> Option<()> {
    let owner = field.class;
    let class = program.class(owner);
    let capture = program.field(field);
    // No other class can spell another object's hidden capture field. Its
    // constructors can be called with qualified new only at checked sites.
    let caller = program.class(id);
    let mut constructors = HashSet::default();
    for (cp, entry) in caller.constant_pool.entries.iter().enumerate() {
        let (Constant::Fieldref { class_index, .. } | Constant::Methodref { class_index, .. }) =
            entry
        else {
            continue;
        };
        if caller.constant_pool.class_name(*class_index).ok() != Some(&class.name) {
            continue;
        }
        let member = caller.constant_pool.member_ref(cp as u16).ok()?;
        if id != owner && member.name == capture.name && member.descriptor == capture.descriptor {
            return None;
        }
        if member.name == "<init>" {
            constructors.insert(cp as u16);
        }
    }
    if constructors.is_empty() {
        return Some(());
    }
    for (index, method) in caller.methods.iter().enumerate() {
        let Some(code) = method
            .code
            .as_ref()
            .filter(|code| code.bytes.contains(&0xb7))
        else {
            continue;
        };
        let ops = decode(&code.bytes).ok()?;
        let mut calls = ops
            .iter()
            .enumerate()
            .filter(|(_, op)| {
                op.opcode == 0xb7
                    && op
                        .operand
                        .constant_index()
                        .is_some_and(|cp| constructors.contains(&cp))
            })
            .peekable();
        if calls.peek().is_none() {
            continue;
        }
        if ops.iter().any(|op| matches!(op.opcode, 0xa8 | 0xa9 | 0xc9)) {
            return None;
        }
        // The allocation and its null check must share every incoming path and handler.
        let cfg = crate::cfg::build(&ops, &code.exception_table).ok()?;
        for (end, _) in calls {
            let start = cfg.blocks[cfg.instruction_blocks[end]].start;
            let site = qualified_site(program, caller, &ops[start..=end], &class.name)?;
            if let Some(pc) = site.guard {
                // DUP supplies the exact same reference to the null check and NEW.
                // Its originating slot can be an ordinary local or a reassigned
                // parameter. Argument binding already applies the constructor's type.
                guards.push((MemberId { class: id, index }, pc));
            } else if site.slot != 0
                || method.access_flags & STATIC != 0
                || !program.is_subtype(&caller.name, &program.class(enclosing).name)
                || ops
                    .iter()
                    .any(|op| op.opcode == 0x3a && op.operand == Operand::Local(0))
            {
                // An unguarded ALOAD is non-null only for the original receiver.
                return None;
            }
        }
    }
    Some(())
}

fn aload(instruction: &Instruction, slot: u16) -> bool {
    instruction.opcode == 0x19 && instruction.operand == Operand::Local(slot)
}

fn references(
    class: &ClassFile,
    op: &Instruction,
    owner: &str,
    name: &str,
    descriptor: &str,
) -> bool {
    op.member_ref(class)
        .is_some_and(|m| m.owner == owner && m.name == name && m.descriptor == descriptor)
}

struct Qualification {
    slot: u16,
    guard: Option<u32>,
}

fn qualified_site(
    program: &Program<'_>,
    class: &ClassFile,
    ops: &[Instruction],
    target: &str,
) -> Option<Qualification> {
    let (call, prefix) = ops.split_last()?;
    let descriptor = MethodDescriptor::parse(call.member_ref(class)?.descriptor).ok()?;
    let (_, explicit) = descriptor.parameters.split_first()?;
    let (prefix, arguments) = prefix.split_at(prefix.len().checked_sub(explicit.len())?);
    if !arguments.iter().all(|op| matches!(op.opcode, 0x01..=0x19)) {
        return None;
    }
    let (allocation, guard) = match prefix {
        [allocation @ .., dup, check, pop] if dup.opcode == 0x59 && pop.opcode == 0x57 => {
            let member = check.member_ref(class)?;
            match (check.opcode, member.owner, member.name, member.descriptor) {
                (0xb6, "java/lang/Object", "getClass", "()Ljava/lang/Class;") => {}
                (
                    0xb8,
                    "java/util/Objects",
                    "requireNonNull",
                    "(Ljava/lang/Object;)Ljava/lang/Object;",
                ) if program
                    .id(member.owner)
                    .is_none_or(|id| !program.is_own(id)) => {}
                _ => return None,
            }
            (allocation, Some(check.pc))
        }
        _ => (prefix, None),
    };
    let [.., new, dup, load] = allocation else {
        return None;
    };
    if (new.opcode, dup.opcode, load.opcode) != (0xbb, 0x59, 0x19)
        || class
            .constant_pool
            .class_name(new.operand.constant_index()?)
            .ok()?
            != target
    {
        return None;
    }
    Some(Qualification {
        slot: load.local_index().ok()?,
        guard,
    })
}

/// The qualified allocation supplies this exact null check at the same
/// expression boundary. A retained getClass/requireNonNull would check twice.
pub(super) fn lower(cx: &EmissionContext<'_, '_>, member: MemberId, ir: &mut MethodIr) {
    let class = cx.program.class(member.class);
    let guards = cx.types.units.qualification_guards(member);
    let capture = cx
        .types
        .units
        .capture(member.class)
        .filter(|_| cx.program.method(member).name == "<init>")
        .map(|field| cx.program.field(field));
    if guards.is_none() && capture.is_none() {
        return;
    }
    for block in &mut ir.blocks {
        block.statements.retain(|stmt| match stmt {
            Statement::PutField { field, .. } => !capture.is_some_and(|f| {
                field.owner == class.name
                    && field.name == f.name
                    && field.descriptor == f.descriptor
            }),
            Statement::Assign { value, .. } => {
                !guards.is_some_and(|g| g.contains(&ir.values[value.value()].pc))
            }
            Statement::Eval(value) => !guards.is_some_and(|g| g.contains(&ir.values[*value].pc)),
            _ => true,
        });
    }
}
