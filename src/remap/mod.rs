//! Bytecode naming is a separate output contract from Java source recovery.
//!
//! Original declarations and hierarchy resolve every reference. Existing access
//! flags, method PCs, exception ranges, stack maps, and unknown attribute payloads
//! retain their original form. Shared ClassName consumers use same-width call or
//! field adapters; their appended methods carry the appropriate verifier maps.
//! Source-only helpers and semantic domains never become bytecode declarations.

mod archive;
mod class;
mod literals;

use crate::{Result, mappings::MappingPlan, naming::NamePlan, program::Program};
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct RemapReport {
    pub classes: usize,
    pub resources: usize,
    pub warnings: Vec<String>,
}

/// Write a JAR from the original bytes retained by [`crate::input::Input::read_for_remap`].
/// Build the program with [`Program::from_input`] to borrow those bytes and classes.
/// The destination is replaced atomically only after every class succeeds.
pub fn remap_jar(
    destination: &Path,
    program: &Program<'_>,
    names: &NamePlan,
) -> Result<RemapReport> {
    remap_jar_with_mappings(destination, program, names, &MappingPlan::default())
}

/// Like [`remap_jar`], including explicitly contracted `@ClassName` literals.
/// Reflection literals use the same semantic analysis as source emission.
pub fn remap_jar_with_mappings(
    destination: &Path,
    program: &Program<'_>,
    names: &NamePlan,
    mappings: &MappingPlan,
) -> Result<RemapReport> {
    archive::write(destination, program, names, mappings)
}

fn descriptor(original: &str, names: &NamePlan) -> Result<String> {
    // Validate executable descriptors independently of optional generic metadata.
    if original.starts_with('(') {
        crate::classfile::MethodDescriptor::parse(original)?;
    } else {
        crate::classfile::JvmType::parse(original)?;
    }
    let mut result = String::with_capacity(original.len());
    let mut rest = original;
    while let Some(start) = rest.find('L') {
        result.push_str(&rest[..=start]);
        rest = &rest[start + 1..];
        let end = rest
            .find(';')
            .ok_or_else(|| anyhow::anyhow!("unterminated descriptor {original}"))?;
        result.push_str(names.class_name_for(&rest[..end]));
        result.push(';');
        rest = &rest[end + 1..];
    }
    result.push_str(rest);
    Ok(result)
}
