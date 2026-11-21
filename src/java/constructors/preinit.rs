//! Determine when pre-super capture stores may be emitted in the Java constructor body.
use super::*;
use crate::{
    classfile::{STATIC, VOLATILE},
    ir::Literal,
    program::ClassId,
};

fn input(ir: &MethodIr, value: ValueId) -> bool {
    matches!(
        ir.values[value].kind,
        ValueKind::Parameter { index: Some(_), .. }
            | ValueKind::Literal(
                Literal::Null
                    | Literal::Int(_)
                    | Literal::Long(_)
                    | Literal::Float(_)
                    | Literal::Double(_)
            )
    )
}

pub(super) fn plain_receiver_store(
    program: &Program<'_>,
    owner: ClassId,
    ir: &MethodIr,
    statement: &Statement,
) -> bool {
    let Statement::PutField {
        field,
        receiver: Some(receiver),
        value,
        ..
    } = statement
    else {
        return false;
    };
    field.owner == program.class(owner).name
        && ir.aliases_this(*receiver)
        && input(ir, *value)
        && program
            .declared_field(owner, &field.name, &field.descriptor)
            .is_some_and(|id| program.field(id).access_flags & (STATIC | VOLATILE) == 0)
}

/// An unpublished receiver is unobservable across Object() and constructors
/// which only copy arguments or constants into their own fields. Inspect the
/// actual called overload at every step. API declarations have no executable
/// contract, and a virtual call could expose the subclass before its stores.
pub(super) fn can_delay_stores(
    cx: &EmissionContext<'_, '_>,
    ir: &MethodIr,
    call: &Delegation,
) -> bool {
    // Arguments can call other objects before delegation. Until Object()
    // returns even a throwing argument cannot expose this through finalization.
    // Preserve the local inputs which the delayed field assignments will read.
    if call.args.iter().any(|&arg| {
        crate::ir::scheduled::values(ir, ValueUse::Reference(arg)).any(|usage| {
            let kind = &ir.values[usage.value()].kind;
            matches!(kind, ValueKind::Parameter { index: None, .. })
                || usage
                    .local(ir)
                    .is_some_and(|local| ir.locals[local].is_this)
                || (usage.local(ir).is_none() && kind.local_write().is_some())
        })
    }) {
        return false;
    }
    let program = cx.program;
    let mut owner = cx.types.owner;
    let mut target = call.target.clone();
    let mut visited = HashSet::default();
    loop {
        // Stay out of the declaring subclass: one of its other constructors
        // could overwrite the field whose store we want to delay. Within a
        // superclass, this(...) just selects another body to check.
        let class = program.class(owner);
        let same_parent = owner != cx.types.owner && target.owner == class.name;
        if !same_parent && class.super_name.as_deref() != Some(&target.owner) {
            return false;
        }
        let target_class = program.id(&target.owner);
        if target.owner == "java/lang/Object" {
            return target.descriptor == "()V" && target_class.is_none_or(|id| !program.is_own(id));
        }
        let Some(parent) = target_class.filter(|&id| program.is_own(id)) else {
            return false;
        };
        let Some(member) = program.declared_method(parent, "<init>", &target.descriptor) else {
            return false;
        };
        if !visited.insert(member) {
            return false;
        }
        let Ok((body, _)) = cx.method_ir(member) else {
            return false;
        };
        let Some(next) = field_initializing_delegation(program, parent, &body) else {
            return false;
        };
        owner = parent;
        target = next;
    }
}

fn field_initializing_delegation(
    program: &Program<'_>,
    owner: ClassId,
    ir: &MethodIr,
) -> Option<Member> {
    let mut next = None;
    let mut seen = HashSet::default();
    let mut block = ir.entry;
    loop {
        let body = &ir.blocks[block];
        if !body.handlers.is_empty() || !seen.insert(block) {
            return None;
        }
        for statement in &body.statements {
            match statement {
                Statement::ConstructorCall {
                    method,
                    receiver,
                    args,
                    ..
                } if next.is_none()
                    && ir.aliases_this(*receiver)
                    && args.iter().all(|&value| input(ir, value)) =>
                {
                    next = Some(method.clone())
                }
                statement if plain_receiver_store(program, owner, ir, statement) => {}
                _ => return None,
            }
        }
        match body.terminator {
            Terminator::Goto(target) => block = target,
            Terminator::Return(None) => return next,
            _ => return None,
        }
    }
}
