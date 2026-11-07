//! Export-only compiler placeholders for absent external classes.
//!
//! Decoded bytecode proves allocation, descriptors and invocation kinds. It
//! does not prove a superclass, interfaces, visibility or checked exceptions.
//! None of these classfiles may enter the semantic declaration set.
use super::{
    requirements::{Member, Requirements},
    schema, writer,
};
use crate::{
    Result,
    classfile::{JvmType, MethodDescriptor},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(serde::Serialize)]
pub(super) struct CompileClass {
    #[serde(skip)]
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub bytes: Vec<u8>,
    kind: &'static str,
    evidence: &'static str,
    unknown: Vec<&'static str>,
    observed_members: Vec<Member>,
}

pub(super) fn build(
    requirements: &Requirements,
    known: impl Fn(&str) -> bool,
) -> Result<(BTreeMap<String, CompileClass>, Vec<String>)> {
    let owners: BTreeSet<_> = requirements
        .instantiated_classes
        .iter()
        .chain(&requirements.superclasses)
        .chain(&requirements.signature_types)
        .chain(requirements.members.iter().filter_map(|(owner, members)| {
            members
                .iter()
                .any(|member| member.is_static.is_some())
                .then_some(owner)
        }))
        .filter(|owner| !known(owner))
        .cloned()
        .collect();
    let mut candidates = BTreeMap::new();
    let mut warnings = Vec::new();
    for owner in owners {
        let members: Vec<_> = requirements
            .members
            .get(&owner)
            .into_iter()
            .flatten()
            .filter(|member| member.is_static.is_some())
            .cloned()
            .collect();
        let allocated = requirements.instantiated_classes.contains(&owner);
        let invoked = requirements.invoked_classes.contains(&owner);
        let superclass = requirements.superclasses.contains(&owner);
        let opaque = !allocated
            && !invoked
            && !superclass
            && members.is_empty()
            && requirements.signature_types.contains(&owner)
            && !requirements.declared_interfaces.contains(&owner)
            && !requirements.invoked_interfaces.contains(&owner)
            && !requirements.exception_types.contains(&owner);
        let invalid = if !allocated && !invoked && !superclass && !opaque {
            Some("no decoded allocation or class invocation proves the class kind".to_owned())
        } else if requirements.invoked_interfaces.contains(&owner)
            || requirements.declared_interfaces.contains(&owner)
        {
            Some("conflicting class evidence and interface invocation".to_owned())
        } else {
            validate(&members).err()
        };
        if let Some(reason) = invalid {
            warnings.push(format!(
                "no API declaration for {owner}; compile stub not inferred: {reason}"
            ));
        } else {
            let mut dependencies = Requirements::default();
            for member in &members {
                dependencies.descriptor(&member.descriptor);
            }
            let evidence = if allocated {
                "decoded new instruction and observed member references"
            } else if invoked {
                "decoded class invocation and observed member references"
            } else if superclass {
                "declared superclass and observed inherited member references"
            } else {
                "descriptor-only type; opaque class shape for compilation"
            };
            candidates.insert(owner, (members, dependencies.classes, evidence, opaque));
        }
    }
    // Do not export a superficially complete placeholder whose signatures need
    // another unknown type that cannot itself be inferred under these rules.
    loop {
        let unsupported: Vec<_> = candidates
            .iter()
            .filter_map(|(owner, (_, dependencies, _, _))| {
                dependencies
                    .iter()
                    .find(|name| !known(name) && !candidates.contains_key(*name))
                    .map(|name| (owner.clone(), name.clone()))
            })
            .collect();
        if unsupported.is_empty() {
            break;
        }
        for (owner, dependency) in unsupported {
            candidates.remove(&owner);
            warnings.push(format!("no API declaration for {owner}; compile stub not inferred: unresolved signature type {dependency}"));
        }
    }
    let mut classes = BTreeMap::new();
    for (owner, (members, _, evidence, opaque)) in candidates {
        let mut declaration = schema::Class {
            name: owner.clone(),
            version: 196653,
            access: 0x21,
            super_name: Some("java/lang/Object".into()),
            interfaces: vec![],
            inner_classes: vec![],
            signature: None,
            enclosing: None,
            fields: vec![],
            methods: vec![],
        };
        for member in &members {
            let access = 1 | if member.is_static == Some(true) { 8 } else { 0 };
            if member.field {
                declaration.fields.push(schema::Field {
                    name: member.name.clone(),
                    descriptor: member.descriptor.clone(),
                    access,
                    signature: None,
                    constant: None,
                });
            } else {
                declaration.methods.push(schema::Method {
                    name: member.name.clone(),
                    descriptor: member.descriptor.clone(),
                    access,
                    signature: None,
                    exceptions: vec![],
                });
            }
        }
        classes.insert(
            owner.clone(),
            CompileClass {
                bytes: writer::class_bytes(&declaration)?,
                kind: "bytecode-inferred compile-only placeholder",
                evidence,
                unknown: [
                    "superclass",
                    "interfaces",
                    "checked exceptions",
                    "visibility",
                    "other modifiers",
                ]
                .into_iter()
                .chain(opaque.then_some("class kind"))
                .collect(),
                observed_members: members,
            },
        );
        warnings.push(format!("bytecode-inferred compile stub for {owner}; hierarchy, throws and visibility remain unknown to analysis"));
    }
    Ok((classes, warnings))
}

fn validate(members: &[Member]) -> std::result::Result<(), String> {
    let mut signatures = BTreeMap::new();
    for member in members {
        let key = if member.field {
            match JvmType::parse(&member.descriptor) {
                Ok(JvmType::Void) | Err(_) => return Err("invalid field descriptor".into()),
                Ok(_) => {}
            }
            (true, member.name.clone())
        } else {
            let descriptor = MethodDescriptor::parse(&member.descriptor)
                .map_err(|_| "invalid method descriptor")?;
            if member.name == "<init>" {
                if member.is_static != Some(false) || descriptor.return_type != JvmType::Void {
                    return Err("invalid constructor reference".into());
                }
            } else if member.name.starts_with('<') {
                return Err("unsupported special method reference".into());
            }
            let parameters_end = member.descriptor.find(')').unwrap();
            (
                false,
                format!("{}{}", member.name, &member.descriptor[..=parameters_end]),
            )
        };
        let shape = (&member.descriptor, member.is_static);
        if signatures
            .insert(key, shape)
            .is_some_and(|old| old != shape)
        {
            return Err(format!(
                "conflicting descriptor or staticness for {}",
                member.name
            ));
        }
    }
    Ok(())
}
