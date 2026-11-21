//! Compiler-owned enclosing instances of named member classes.
use super::context::EmissionContext;
use super::units::TypeUsers;
use crate::{
    bytecode::decode,
    classfile::{
        Constant, FINAL, INTERFACE, MethodDescriptor, PROTECTED, PUBLIC, STATIC, VOLATILE,
    },
    hash::{HashMap, HashSet},
    ir::{InvokeKind, MethodIr, Statement, Terminator, ValueId, ValueKind, ValueUse},
    naming::NamePlan,
    program::{ClassId, MemberId, Program},
};

pub(super) struct Capture {
    pub field: MemberId,
    pub guards: Vec<(MemberId, u32, u32)>,
}

pub(super) fn capture(
    program: &Program<'_>,
    names: &NamePlan,
    owner: ClassId,
    enclosing: ClassId,
    users: &TypeUsers,
) -> Option<Capture> {
    let class = program.class(owner);
    // Extending another true inner class needs qualified super syntax. A
    // displaced NEW of a class with observable static initialization can also
    // require a static trigger, which a Java 1.3 inner class cannot declare.
    let parent = class.super_name.as_deref()?;
    if program
        .id(parent)
        .and_then(|id| program.enclosing_class(id))
        .is_some_and(|(_, entry, _)| entry.access_flags & STATIC == 0)
        || !crate::initialization::superclass_initialization_is_quiet(program, owner)
        || program.descendants(owner).len() > 1
        || program.class(enclosing).access_flags & INTERFACE != 0
    {
        return None;
    }
    // Tools are free to rename synthetic fields. Its type, flags and the
    // constructor writes checked below identify the enclosing instance.
    let descriptor = format!("L{};", program.class(enclosing).name);
    let mut captures = class.fields.iter().enumerate().filter(|(_, f)| {
        f.synthetic
            && f.descriptor == descriptor
            && f.access_flags & (PUBLIC | PROTECTED | FINAL | STATIC | VOLATILE) == FINAL
    });
    let field = MemberId {
        class: owner,
        index: captures.next()?.0,
    };
    if captures.next().is_some()
        || names.is_mapped_field(field)
        || !class.methods.iter().any(|m| m.name == "<init>")
        || class.methods.iter().any(|method| {
            method.access_flags & STATIC != 0
                || method.name == "<clinit>"
                || method.name == "<init>" && method.code.is_none()
        })
        || class.fields.iter().enumerate().any(|(index, field)| {
            field.access_flags & STATIC != 0
                && (field.access_flags & FINAL == 0
                    || field.constant_value.is_none()
                    || !program.initialization_plan().permits_final(MemberId {
                        class: owner,
                        index,
                    }))
        })
    {
        return None;
    }
    let mut guards = Vec::new();
    for &user in users.of(program, owner) {
        check_uses(program, names, user, field, enclosing, &mut guards)?;
    }
    Some(Capture { field, guards })
}

/// The constant pool only selects methods to inspect. The scheduled graph
/// supplies the actual receiver, hidden argument and explicit arguments; no
/// reconstruction of the JVM operand stack belongs in the Java unit planner.
fn check_uses(
    program: &Program<'_>,
    names: &NamePlan,
    user: ClassId,
    field: MemberId,
    enclosing: ClassId,
    guards: &mut Vec<(MemberId, u32, u32)>,
) -> Option<()> {
    let target = &program.class(field.class).name;
    let captured = program.field(field);
    let class = program.class(user);
    let mut refs = HashSet::default();
    for (index, constant) in class.constant_pool.entries.iter().enumerate() {
        if matches!(
            constant,
            Constant::Methodref { .. } | Constant::Fieldref { .. }
        ) {
            let member = class.constant_pool.member_ref(index as u16).ok()?;
            if member.owner == target
                && (member.name == "<init>"
                    || member.name == captured.name && member.descriptor == captured.descriptor)
            {
                refs.insert(index as u16);
            }
        }
    }
    if refs.is_empty() && user != field.class {
        return Some(());
    }
    for (index, method) in class.methods.iter().enumerate() {
        let Some(code) = &method.code else { continue };
        let constructor = user == field.class && method.name == "<init>";
        if !constructor && !code.bytes.iter().any(|op| matches!(op, 0xb2..=0xb5 | 0xb7)) {
            continue;
        }
        let ops = decode(&code.bytes).ok()?;
        let uses: Vec<_> = ops
            .iter()
            .enumerate()
            .filter_map(|(i, op)| {
                (matches!(op.opcode, 0xb2..=0xb5 | 0xb7)
                    && op
                        .operand
                        .constant_index()
                        .is_some_and(|cp| refs.contains(&cp)))
                .then_some(i)
            })
            .collect();
        if !constructor && uses.is_empty() {
            continue;
        }
        // The plan matches guard identities between the original and scheduled
        // graph. A cloned subroutine can produce multiple identities per PC.
        if ops.iter().any(|op| matches!(op.opcode, 0xa8 | 0xa9)) {
            return None;
        }
        let member = MemberId { class: user, index };
        let prepared = crate::cfg::PreparedCode::from_decoded(code, ops).ok()?;
        let (ir, _) = program.lift_prepared(member, &prepared, &[], true).ok()?;
        let store = if constructor {
            if names
                .parameter_name(member, 0)
                .is_some_and(|name| !name.is_empty())
                || MethodDescriptor::parse(&method.descriptor)
                    .ok()?
                    .parameters
                    .first()?
                    .descriptor()
                    != captured.descriptor
                || prepared
                    .instructions
                    .iter()
                    .any(|op| op.opcode == 0x3a && op.local_index().ok() == Some(1))
            {
                return None;
            }
            Some(capture_store(program, field, &ir)?)
        } else {
            None
        };
        // Multiple scheduled values may share a PC (parameters and phis in
        // particular). Only the instructions relevant to this proof are indexed.
        let operations: HashMap<_, _> = ir
            .values
            .iter()
            .filter(|v| {
                matches!(
                    v.kind,
                    ValueKind::Field { .. } | ValueKind::New { .. } | ValueKind::Call { .. }
                )
            })
            .map(|v| (v.pc, &v.kind))
            .collect();
        for i in uses {
            let op = &prepared.instructions[i];
            match (op.opcode, operations.get(&op.pc)) {
                (
                    0xb4,
                    Some(ValueKind::Field {
                        receiver: Some(r), ..
                    }),
                ) if user == field.class && ir.aliases_this(*r) => {}
                (0xb5, _) if Some(op.pc) == store => {}
                (
                    0xb7,
                    Some(ValueKind::New {
                        class: allocated,
                        args,
                        allocation_pc,
                        ..
                    }),
                ) if allocated == target => {
                    let &outer = args.first()?;
                    if ir.aliases_this(outer)
                        && program.is_subtype(&class.name, &program.class(enclosing).name)
                    {
                        continue;
                    }
                    let block = &prepared.graph.blocks[prepared.graph.instruction_blocks[i]];
                    let guard = prepared.instructions[block.start..i]
                        .windows(2)
                        .rev()
                        .find_map(|pair| {
                            let check = &pair[0];
                            (check.pc > *allocation_pc
                                && matches!(check.opcode, 0xb6 | 0xb8)
                                && pair[1].opcode == 0x57
                                && operations
                                    .get(&check.pc)
                                    .and_then(|kind| null_checked(program, kind))
                                    == Some(outer))
                            .then_some(check.pc)
                        })?;
                    guards.push((member, guard, *allocation_pc));
                }
                _ => return None,
            }
        }
    }
    Some(())
}

fn null_checked(program: &Program<'_>, kind: &ValueKind) -> Option<ValueId> {
    let ValueKind::Call {
        kind,
        method,
        receiver,
        args,
    } = kind
    else {
        return None;
    };
    match (
        *kind,
        method.owner.as_str(),
        method.name.as_str(),
        method.descriptor.as_str(),
    ) {
        (InvokeKind::Virtual, "java/lang/Object", "getClass", "()Ljava/lang/Class;") => *receiver,
        (
            InvokeKind::Static,
            "java/util/Objects",
            "requireNonNull",
            "(Ljava/lang/Object;)Ljava/lang/Object;",
        ) if program.can_use_platform_contract(&method.owner) => args.first().copied(),
        _ => None,
    }
}

fn capture_store(program: &Program<'_>, capture_id: MemberId, ir: &MethodIr) -> Option<u32> {
    // javac versions store the capture on different sides of super(...).
    // The superclass must not read or publish the partially initialized child.
    let class = program.class(capture_id.class);
    let capture = program.field(capture_id);
    let mut block = ir.entry;
    let mut write = None;
    let mut delegated = false;
    let mut seen = HashSet::default();
    while seen.insert(block) && ir.blocks[block].handlers.is_empty() {
        for stmt in &ir.blocks[block].statements {
            match stmt {
                Statement::PutField {
                    pc: Some(pc),
                    field,
                    receiver: Some(r),
                    value,
                } if write.is_none()
                    && field.owner == class.name
                    && field.name == capture.name
                    && field.descriptor == capture.descriptor
                    && ir.aliases_this(*r)
                    && *value == ir.parameters[1] =>
                {
                    write = Some(*pc)
                }
                Statement::ConstructorCall {
                    method, receiver, ..
                } if !delegated
                    && ir.aliases_this(*receiver)
                    && program.super_preserves_receiver(capture_id.class, method) =>
                {
                    delegated = true
                }
                _ => return None,
            }
            if delegated && write.is_some() {
                return write;
            }
        }
        let Terminator::Goto(next) = ir.blocks[block].terminator else {
            break;
        };
        block = next;
    }
    None
}

pub(super) const SUPPORT_PREFIX: &str = "$sf$Support";

/// Helpers for one member share a lexical sibling. The reserved prefix
/// excludes authored and debug locals. Check types and every visible field
/// because the container is used both as a type and as a call qualifier.
pub(super) fn support_name(program: &Program<'_>, types: &super::types::TypeNames<'_>) -> String {
    let mut name = format!("{SUPPORT_PREFIX}{}", types.owner);
    loop {
        let suffix = format!("${name}");
        let occupied = types.local_reservations().contains(&name)
            || program.classes().iter().enumerate().any(|(id, _)| {
                types.names.class_name(id).ends_with(&suffix)
                    || crate::naming::simple_name(types.names.class_name(id)) == name
            })
            || types.units.scopes(program, types.owner).any(|scope| {
                (0..program.class(scope).fields.len()).any(|index| {
                    types.names.field_name(MemberId {
                        class: scope,
                        index,
                    }) == name
                })
            });
        if !occupied {
            return name;
        }
        name.push('_');
    }
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
    let root = |stmt: &Statement| match stmt {
        Statement::Assign { value, .. } => Some(*value),
        Statement::Eval(id) => Some(ValueUse::Definition(*id)),
        _ => None,
    };
    let mut elided = HashSet::default();
    if let Some(guards) = guards {
        let new_at = |use_: ValueUse| match ir.values[use_.value()].kind {
            ValueKind::New { allocation_pc, .. } if use_.local(ir).is_none() => Some(allocation_pc),
            _ => None,
        };
        for block in &ir.blocks {
            // An intervening statement may throw or have an effect. Keep the
            // explicit check unless the very next expression supplies it.
            let mut next_new = match block.terminator {
                Terminator::Return(Some(id)) | Terminator::Throw { value: id, .. } => {
                    new_at(ValueUse::Reference(id))
                }
                _ => None,
            };
            for stmt in block.statements.iter().rev() {
                let value = root(stmt);
                if let Some(use_) = value
                    && use_.local(ir).is_none()
                    && let Some(&pc) = guards.get(&ir.values[use_.value()].pc)
                    && Some(pc) == next_new
                {
                    elided.insert(use_.value());
                } else {
                    next_new = value.and_then(new_at);
                }
            }
        }
    }
    let mut captured_field = None;
    for block in &mut ir.blocks {
        block.statements.retain(|stmt| {
            if let Statement::PutField { field, .. } = stmt
                && capture.is_some_and(|f| {
                    field.owner == class.name
                        && field.name == f.name
                        && field.descriptor == f.descriptor
                })
            {
                captured_field = Some(field.clone());
                false
            } else {
                !root(stmt).is_some_and(|use_| elided.contains(&use_.value()))
            }
        });
    }
    if let Some(field) = captured_field {
        // javac can read the incoming outer instance directly in field
        // initializers. Java supplies the same reference as Outer.this. Keep
        // the JVM signature intact and bind uses of its hidden input in the IR.
        let outer = ir.parameters[1];
        let read = ir.push_value(
            ValueKind::Field {
                field,
                receiver: Some(ir.parameters[0]),
            },
            ir.values[outer].ty.clone(),
            ir.entry,
            0,
            None,
        );
        let bind = |id| if id == outer { read } else { id };
        for value in &mut ir.values {
            value.kind.map_operands(bind);
        }
        for block in &mut ir.blocks {
            for statement in &mut block.statements {
                statement.map_operands(bind);
            }
            block.terminator.map_operands(bind);
        }
    }
}
