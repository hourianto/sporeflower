//! Adapt portable original-identity names into the existing native naming plan.
use super::SessionMappings;
use crate::hash::HashMap;
use crate::{
    Result,
    classfile::MethodDescriptor,
    mappings::MappingPlan,
    program::{ClassId, MemberId, Program},
};
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use std::hash::Hash;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClassRename {
    pub src: String,
    pub dst: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FieldRename {
    pub owner: String,
    pub name: String,
    pub descriptor: Option<String>,
    pub dst: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MethodRename {
    pub owner: String,
    pub name: String,
    pub descriptor: String,
    pub dst: Option<String>,
    #[serde(default)]
    pub parameters: Vec<ParameterRename>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterRename {
    /// JVM local-variable slot; `this` and the second half of wide values are invalid.
    pub index: u16,
    pub name: String,
}

pub(super) fn apply(
    program: &Program<'_>,
    inputs: &SessionMappings,
    mappings: &mut MappingPlan,
) -> Result<()> {
    for rename in &inputs.classes {
        let id = own_class(program, &rename.src)?;
        insert(&mut mappings.names.classes, id, &rename.dst)
            .with_context(|| format!("class mapping {}", rename.src))?;
    }
    for rename in &inputs.fields {
        let class = own_class(program, &rename.owner)?;
        let mut candidates = program
            .class(class)
            .fields
            .iter()
            .enumerate()
            .filter(|(_, field)| {
                field.name == rename.name
                    && rename
                        .descriptor
                        .as_ref()
                        .is_none_or(|descriptor| descriptor == &field.descriptor)
            });
        let (index, _) = candidates.next().with_context(|| {
            format!(
                "field mapping target not found: {}.{}",
                rename.owner, rename.name
            )
        })?;
        ensure!(
            candidates.next().is_none(),
            "ambiguous field mapping {}.{}; supply a descriptor",
            rename.owner,
            rename.name
        );
        insert(
            &mut mappings.names.fields,
            MemberId { class, index },
            &rename.dst,
        )
        .with_context(|| format!("field mapping {}.{}", rename.owner, rename.name))?;
    }
    for rename in &inputs.methods {
        let class = own_class(program, &rename.owner)?;
        let index = program
            .class(class)
            .methods
            .iter()
            .position(|method| method.name == rename.name && method.descriptor == rename.descriptor)
            .with_context(|| {
                format!(
                    "method mapping target not found: {}.{}{}",
                    rename.owner, rename.name, rename.descriptor
                )
            })?;
        let member = MemberId { class, index };
        if let Some(name) = &rename.dst {
            insert(&mut mappings.names.methods, member, name).with_context(|| {
                format!(
                    "method mapping {}.{}{}",
                    rename.owner, rename.name, rename.descriptor
                )
            })?;
        }
        if !rename.parameters.is_empty() {
            parameters(program, member, &rename.parameters, mappings).with_context(|| {
                format!(
                    "parameter mapping {}.{}{}",
                    rename.owner, rename.name, rename.descriptor
                )
            })?;
        }
    }
    Ok(())
}

fn own_class(program: &Program<'_>, name: &str) -> Result<ClassId> {
    program
        .id(name)
        .filter(|&id| program.is_own(id))
        .with_context(|| format!("mapping class is not in session: {name}"))
}

fn insert<K: Eq + Hash>(map: &mut HashMap<K, String>, key: K, name: &str) -> Result<()> {
    if let Some(previous) = map.get(&key) {
        ensure!(
            previous == name,
            "conflicting authored names: {previous:?} and {name:?}"
        );
    } else {
        map.insert(key, name.to_owned());
    }
    Ok(())
}

fn parameters(
    program: &Program<'_>,
    member: MemberId,
    inputs: &[ParameterRename],
    mappings: &mut MappingPlan,
) -> Result<()> {
    let method = program.method(member);
    let descriptor = MethodDescriptor::parse(&method.descriptor)?;
    let mut slot = u16::from(method.access_flags & 0x0008 == 0);
    let slots: Vec<_> = descriptor
        .parameters
        .iter()
        .map(|ty| {
            let current = slot;
            slot += ty.slots();
            current
        })
        .collect();
    let existing = mappings.names.parameters.get(&member);
    let mut names = existing.cloned().unwrap_or_else(|| {
        (0..slots.len())
            .map(|ordinal| format!("arg{ordinal}"))
            .collect()
    });
    let mut authored = HashMap::default();
    for input in inputs {
        let ordinal = slots
            .iter()
            .position(|&slot| slot == input.index)
            .with_context(|| format!("slot {} does not name a parameter", input.index))?;
        if let Some(existing) = existing {
            ensure!(
                existing[ordinal] == input.name,
                "conflicting authored parameter name at slot {}: {:?} and {:?}",
                input.index,
                existing[ordinal],
                input.name
            );
        }
        insert(&mut authored, ordinal, &input.name)?;
        names[ordinal].clone_from(&input.name);
    }
    mappings.names.parameters.insert(member, names);
    Ok(())
}
