//! Source-only members retain their exact identity and source/runtime contract.
//! This registry belongs to the body artifact until its owner commits it.
use super::{exceptions::Thrown, types::TypeNames};
use crate::{
    classfile::{JvmType, MethodDescriptor},
    ir::{BlockId, LocalId, Member, MethodIr, Statement, Value, ValueId, ValueKind},
    program::{MemberId, Program},
};
use std::collections::BTreeMap;

#[derive(Clone)]
pub(super) struct Contract {
    pub signature: MethodDescriptor,
    pub effects: Thrown,
    /// Interface helpers live in an explicitly selected nested container.
    pub container: Option<String>,
}

#[derive(Clone, Default)]
pub(super) struct Members {
    contracts: BTreeMap<String, BTreeMap<String, BTreeMap<String, Contract>>>,
}

impl Members {
    pub fn insert(&mut self, member: &Member, contract: Contract) {
        assert_eq!(member.descriptor, contract.signature.descriptor());
        self.contracts
            .entry(member.owner.clone())
            .or_default()
            .entry(member.name.clone())
            .or_default()
            .insert(member.descriptor.clone(), contract);
    }

    pub fn get(&self, owner: &str, name: &str, descriptor: &str) -> Option<&Contract> {
        self.contracts.get(owner)?.get(name)?.get(descriptor)
    }
}

/// Reserve against source-visible types, original/mapped members, and the
/// binary name of a generated nested class. Both source shadowing and flattened
/// binary-name collisions matter.
pub(super) fn symbol(program: &Program<'_>, types: &TypeNames<'_>, base: &str) -> String {
    symbol_in(program, types, None, base)
}

pub(super) fn symbol_in(
    program: &Program<'_>,
    types: &TypeNames<'_>,
    container: Option<&str>,
    base: &str,
) -> String {
    let owner = if let Some(container) = container {
        format!("{}${container}", types.names.class_name(types.owner))
    } else {
        types.names.class_name(types.owner).to_owned()
    };
    let occupied = |candidate: &str| {
        let binary = format!("{owner}${candidate}");
        types.reserves_type_name(candidate)
            || program
                .classes()
                .iter()
                .enumerate()
                .any(|(id, _)| types.names.class_name(id) == binary)
            || std::iter::once(types.owner)
                .chain(program.ancestors(types.owner).iter().copied())
                .any(|class| {
                    (0..program.class(class).methods.len()).any(|index| {
                        // Synthetic IR calls share the original owner namespace;
                        // a remapped source name does not free its JVM identity.
                        program.class(class).methods[index].name == candidate
                            || types.names.method_name(MemberId { class, index }) == candidate
                    }) || (0..program.class(class).fields.len())
                        .any(|index| types.names.field_name(MemberId { class, index }) == candidate)
                })
    };
    let mut candidate = base.to_owned();
    while occupied(&candidate) {
        candidate.push('_');
    }
    candidate
}

/// Source spelling is shared by original and generated method declarations.
pub(super) fn throws_clause<'a>(
    exceptions: impl IntoIterator<Item = &'a String>,
    types: &TypeNames<'_>,
) -> String {
    let names = exceptions
        .into_iter()
        .map(|name| types.class(name))
        .collect::<Vec<_>>()
        .join(", ");
    if names.is_empty() {
        String::new()
    } else {
        format!(" throws {names}")
    }
}

/// A transport's declaration and every graph access use one typed field layout.
/// Argument order and original local identities stay distinct across extraction.
pub(super) struct Carrier {
    pub name: String,
    arguments: Vec<JvmType>,
    locals: BTreeMap<LocalId, JvmType>,
}

#[derive(Clone, Copy)]
pub(super) enum Slot {
    Argument(usize),
    Local(LocalId),
}

impl Carrier {
    pub fn new(
        name: String,
        arguments: Vec<JvmType>,
        ir: &MethodIr,
        locals: impl IntoIterator<Item = LocalId>,
    ) -> Self {
        Self {
            name,
            arguments,
            locals: locals
                .into_iter()
                .map(|local| (local, ir.locals[local].ty.clone()))
                .collect(),
        }
    }

    pub fn ty(&self) -> JvmType {
        JvmType::Object(self.name.clone())
    }

    fn slot(&self, slot: Slot) -> (String, &JvmType) {
        match slot {
            Slot::Argument(index) => (format!("a{index}"), &self.arguments[index]),
            Slot::Local(local) => (format!("v{local}"), &self.locals[&local]),
        }
    }

    pub fn field(&self, slot: Slot) -> Member {
        let (name, ty) = self.slot(slot);
        Member {
            owner: self.name.clone(),
            name,
            descriptor: ty.descriptor(),
        }
    }

    pub fn store(&self, receiver: ValueId, slot: Slot, value: ValueId) -> Statement {
        Statement::PutField {
            pc: None,
            field: self.field(slot),
            receiver: Some(receiver),
            value,
        }
    }

    pub fn load(
        &self,
        ir: &mut MethodIr,
        receiver: ValueId,
        slot: Slot,
        block: BlockId,
        pc: u32,
    ) -> ValueId {
        ir.values.push(Value {
            local: None,
            kind: ValueKind::Field {
                field: self.field(slot),
                receiver: Some(receiver),
            },
            ty: self.slot(slot).1.clone(),
            block,
            pc,
        })
    }

    pub fn emit(&self, types: &TypeNames<'_>, out: &mut super::body::CodeWriter) {
        out.open(format!("private static final class {} {{", self.name));
        for slot in (0..self.arguments.len())
            .map(Slot::Argument)
            .chain(self.locals.keys().copied().map(Slot::Local))
        {
            let (name, ty) = self.slot(slot);
            out.line(format!("{} {name};", types.ty(ty)));
        }
        out.close("}");
    }
}
