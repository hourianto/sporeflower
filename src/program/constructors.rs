//! Can a pre-delegation store to the receiver wait until a superclass returns?
//! Program-wide results depend on the original bodies, including their retained
//! operations, and do not depend on either source presentation.
use super::{ClassId, MemberId, Program};
use crate::{
    classfile::{STATIC, VOLATILE},
    hash::HashSet,
    ir::{self, Literal, Member, MethodIr, Statement, Terminator, ValueId, ValueKind, ValueUse},
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

fn receiver_store(
    program: &Program<'_>,
    owner: ClassId,
    ir: &MethodIr,
    statement: &Statement,
) -> Option<ValueId> {
    let Statement::PutField {
        field,
        receiver: Some(receiver),
        value,
        ..
    } = statement
    else {
        return None;
    };
    (field.owner == program.class(owner).name
        && ir.aliases_this(*receiver)
        && program
            .declared_field(owner, &field.name, &field.descriptor)
            .is_some_and(|id| program.field(id).access_flags & (STATIC | VOLATILE) == 0))
    .then_some(*value)
}

impl Program<'_> {
    pub(crate) fn plain_receiver_store(
        &self,
        owner: ClassId,
        ir: &MethodIr,
        statement: &Statement,
    ) -> bool {
        receiver_store(self, owner, ir, statement).is_some_and(|rhs| input(ir, rhs))
    }

    /// A sibling constructor may overwrite the subclass's delayed fields. A
    /// superclass may select another overload with this(...), provided every
    /// body on that chain leaves the partially initialized receiver unpublished.
    pub(crate) fn super_preserves_receiver(&self, owner: ClassId, first: &Member) -> bool {
        if self.class(owner).super_name.as_deref() != Some(&first.owner) {
            return false;
        }
        let mut visited = HashSet::<MemberId>::default();
        let mut target = first.clone();
        let safe = loop {
            let class = self.id(&target.owner);
            if target.owner == "java/lang/Object" {
                break target.descriptor == "()V"
                    && self.can_use_platform_contract("java/lang/Object");
            }
            let Some(class) = class.filter(|&id| self.is_own(id)) else {
                break false;
            };
            let Some(member) = self.declared_method(class, "<init>", &target.descriptor) else {
                break false;
            };
            if let Some(&known) = self
                .analyses
                .constructor_receivers
                .read()
                .unwrap()
                .get(&member)
            {
                break known;
            }
            if !visited.insert(member) {
                break false;
            }
            // Compute outside the lock; lifting can request other program facts.
            let Ok(body) = self.original_method_ir(member) else {
                break false;
            };
            let Some(next) = field_initializing_delegation(self, class, &body) else {
                break false;
            };
            let declaration = self.class(class);
            if next.owner != declaration.name
                && declaration.super_name.as_ref() != Some(&next.owner)
            {
                break false;
            }
            target = next;
        };
        if !visited.is_empty() {
            self.analyses
                .constructor_receivers
                .write()
                .unwrap()
                .extend(visited.into_iter().map(|member| (member, safe)));
        }
        safe
    }
}

/// Every accepted write in the parent stores an argument, a
/// constant, or a nonthrowing calculation on those values. In particular this
/// rejects reads or publication of the receiver and calls that could observe it.
fn argument_expression(ir: &MethodIr, root: ValueUse) -> bool {
    ir::scheduled::values(ir, root).all(|usage| {
        let id = usage.value();
        if let Some(local) = usage.local(ir) {
            return !ir.locals[local].is_this
                && !matches!(ir.values[id].kind, ValueKind::Parameter { index: None, .. });
        }
        match &ir.values[id].kind {
            ValueKind::Alias(_)
            | ValueKind::Unary { .. }
            | ValueKind::Compare { .. }
            | ValueKind::Select { .. } => true,
            ValueKind::Binary { .. } => !ir.operation_is_observable(id),
            ValueKind::Cast { ty, .. } => !ty.is_reference(),
            _ => input(ir, id),
        }
    })
}

fn field_initializing_delegation(
    program: &Program<'_>,
    owner: ClassId,
    ir: &MethodIr,
) -> Option<Member> {
    let mut next = None;
    let mut seen = HashSet::default();
    let mut pending = vec![(ir.entry, false)];
    while let Some((block, mut delegated)) = pending.pop() {
        if !seen.insert((block, delegated)) {
            continue;
        }
        let body = &ir.blocks[block];
        if !body.handlers.is_empty() {
            return None;
        }
        for statement in &body.statements {
            match statement {
                Statement::ConstructorCall {
                    method,
                    receiver,
                    args,
                    ..
                } if !delegated
                    && next.is_none_or(|prior| prior == method)
                    && ir.aliases_this(*receiver)
                    && args
                        .iter()
                        .all(|&arg| argument_expression(ir, ValueUse::Reference(arg))) =>
                {
                    delegated = true;
                    next = Some(method);
                }
                Statement::Assign { local, value }
                    if !ir.locals[*local].is_this && argument_expression(ir, *value) => {}
                store
                    if receiver_store(program, owner, ir, store)
                        .is_some_and(|rhs| argument_expression(ir, ValueUse::Reference(rhs))) => {}
                _ => return None,
            }
        }
        match &body.terminator {
            Terminator::Return(None) if delegated => (),
            Terminator::Goto(_) | Terminator::Branch { .. } | Terminator::Switch { .. }
                if body
                    .terminator
                    .operands()
                    .iter()
                    .all(|&value| argument_expression(ir, ValueUse::Reference(value))) =>
            {
                pending.extend(
                    body.terminator
                        .successors()
                        .into_iter()
                        .map(|target| (target, delegated)),
                );
            }
            _ => return None,
        }
    }
    next.cloned()
}
