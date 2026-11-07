//! Authored Java-like mappings, normalized against original JVM identities.
//! Parsing never requires a Java runtime and never rewrites the input program.
mod builtins;
mod contracts;
mod diagnostics;
mod domains;
mod inputs;
mod lexer;
mod model;
mod parser;
mod resolve;
#[cfg(test)]
#[path = "../../tests/unit/mappings.rs"]
mod tests;

use crate::{Result, program::Program};
pub use diagnostics::{MappingDiagnostic, MappingDiagnostics, MappingOrigin, MappingOrigins};
pub use domains::constant_prefix;
pub use inputs::MappingInputs;
pub use model::*;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

/// Load sorted `.map` files from each directory, or individual mapping files.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_paths(
    program: &Program<'_>,
    paths: &[PathBuf],
    include_builtins: bool,
) -> Result<MappingPlan> {
    MappingInputs::read(paths)?.resolve(program, include_builtins)
}

pub fn parse_sources(program: &Program<'_>, sources: &[MappingSource]) -> Result<MappingPlan> {
    let mut parsed = parser::parse_all(sources)?;
    // Authority does not depend on the order supplied by library callers.
    parsed.sort_by_key(|source| !source.builtin);
    resolve::resolve(program, &parsed)
}

/// Parse one file without adding implicit API contracts.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_file(program: &Program<'_>, path: &Path) -> Result<MappingPlan> {
    load_paths(program, &[path.to_owned()], false)
}
