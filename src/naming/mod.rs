//! A single immutable source-name plan over original JVM identities.
//!
//! Renaming never mutates a class file. A member reference is resolved against the
//! original hierarchy before its declaration's source spelling is looked up.

mod allocate;
mod tiny;

pub(crate) use allocate::is_source_parameter;
pub use allocate::{NameOverrides, NamePlan, RenameOptions, is_identifier};
pub use tiny::mapping_text;

pub(crate) fn simple_name(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

pub(crate) fn parameters(descriptor: &str) -> &str {
    descriptor
        .find(')')
        .map_or(descriptor, |end| &descriptor[..=end])
}
