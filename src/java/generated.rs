//! Source-only members retain their exact identity and source/runtime contract.
//! This registry belongs to the body artifact until its owner commits it.
use super::{exceptions::Thrown, types::TypeNames};
use crate::{
    classfile::MethodDescriptor,
    ir::Member,
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
