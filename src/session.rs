//! In-memory project API for native and browser hosts.
//!
//! Original classes are parsed once. Preparing a session selects bundled APIs,
//! resolves mappings and allocates names once for every subsequent class request.
//! A session owns the data borrowed by its immutable Program without leaking it.

use crate::{
    Result,
    api::{ApiCatalog, ApiOptions},
    classfile::ClassFile,
    java::{DecompiledClass, JavaOptions, decompile_class, domain_sources},
    mappings::{MappingInputs, MappingPlan, MappingSource},
    naming::{NamePlan, RenameOptions},
    program::Program,
};
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod renames;
#[cfg(test)]
mod tests;
pub use renames::{ClassRename, FieldRename, MethodRename, ParameterRename};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionOptions {
    pub rename_obfuscated: bool,
    pub rename_comments: bool,
    pub builtin_mappings: bool,
    pub bundled_api: bool,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            rename_obfuscated: false,
            rename_comments: true,
            builtin_mappings: true,
            bundled_api: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MappingText {
    pub path: String,
    pub source: String,
}

/// Portable name mappings bind original JVM identities, just like authored maps.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SessionMappings {
    pub sources: Vec<MappingText>,
    pub classes: Vec<ClassRename>,
    pub fields: Vec<FieldRename>,
    pub methods: Vec<MethodRename>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionClass {
    pub name: String,
    pub source_name: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMetadata {
    pub classes: Vec<SessionClass>,
    pub diagnostics: Vec<String>,
    pub api_classes: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct DomainSource {
    pub name: String,
    pub source: String,
}

struct Classes {
    own: Vec<ClassFile>,
    libraries: Vec<ClassFile>,
}

struct Prepared<'a> {
    program: Program<'a>,
    mappings: MappingPlan,
    names: NamePlan,
    metadata: SessionMetadata,
}

self_cell::self_cell!(
    struct PreparedCell {
        owner: Classes,
        #[covariant]
        dependent: Prepared,
    }
);

/// A single mutable project, normally confined to one browser Web Worker.
#[derive(Default)]
pub struct Session {
    classes: Vec<ClassFile>,
    manifest: BTreeMap<String, String>,
    mapping_inputs: MappingInputs,
    mappings: SessionMappings,
    options: Option<SessionOptions>,
    prepared: Option<PreparedCell>,
}

impl Session {
    /// Parse bytecode without consulting the filesystem. Entry paths label errors;
    /// class identity always comes from the bytecode itself.
    pub fn add_class(&mut self, path: &str, bytes: &[u8]) -> Result<String> {
        ensure!(
            bytes.len() <= 64 * 1024 * 1024,
            "class {path} exceeds 64 MiB"
        );
        let class = ClassFile::parse(bytes).with_context(|| format!("parse class {path}"))?;
        let classes = self
            .prepared
            .as_ref()
            .map_or(self.classes.as_slice(), |cell| &cell.borrow_owner().own);
        ensure!(
            !classes.iter().any(|existing| existing.name == class.name),
            "duplicate class identity {} ({path})",
            class.name
        );
        self.invalidate();
        let name = class.name.clone();
        self.classes.push(class);
        Ok(name)
    }

    pub fn set_manifest(&mut self, text: &str) {
        self.invalidate();
        self.manifest = crate::input::parse_manifest(text);
    }

    /// Parse authored text immediately. Resolution against the complete class
    /// hierarchy occurs in prepare; syntax errors retain filename/line context.
    pub fn set_mappings(&mut self, mappings: SessionMappings) -> Result<()> {
        let sources: Vec<_> = mappings
            .sources
            .iter()
            .map(|source| MappingSource::new(&source.path, &source.source))
            .collect();
        let inputs = MappingInputs::from_sources(&sources)?;
        self.invalidate();
        self.mapping_inputs = inputs;
        self.mappings = mappings;
        Ok(())
    }

    /// Repeated prepare calls with unchanged inputs/options reuse all analysis.
    /// Failed preparation retains parsed classes so callers can repair mappings.
    pub fn prepare(&mut self, options: SessionOptions) -> Result<&SessionMetadata> {
        if self.options.as_ref() == Some(&options) {
            return Ok(&self.ready()?.metadata);
        }
        self.invalidate();
        ensure!(!self.classes.is_empty(), "no classes in session");
        self.classes.sort_by(|a, b| a.name.cmp(&b.name));
        let mut diagnostics: Vec<_> = self
            .classes
            .iter()
            .flat_map(|class| {
                class
                    .warnings
                    .iter()
                    .map(|warning| format!("{}: {warning}", class.name))
            })
            .collect();
        let libraries = if options.bundled_api {
            let mut selection = ApiCatalog::bundled()?.select(
                &self.classes,
                &ApiOptions {
                    configuration: self.manifest.get("microedition-configuration").cloned(),
                    additional_roots: self
                        .mapping_inputs
                        .required_api_roots(&self.classes, options.builtin_mappings)?,
                    ..ApiOptions::default()
                },
            )?;
            diagnostics.append(&mut selection.warnings);
            selection.classes
        } else {
            Vec::new()
        };
        let classes = Classes {
            own: std::mem::take(&mut self.classes),
            libraries,
        };
        let prepared = PreparedCell::try_new_or_recover(classes, |classes| -> Result<_> {
            let mut program = Program::with_scope(
                &classes.own,
                &classes.libraries,
                crate::input::analysis_scope(&classes.own, &self.manifest),
            )?;
            program.prepare_analysis();
            let mut mappings = self
                .mapping_inputs
                .resolve(&program, options.builtin_mappings)?;
            renames::apply(&program, &self.mappings, &mut mappings)?;
            let names = NamePlan::build_with_mapping_origins(
                &program,
                &RenameOptions {
                    rename_obfuscated: options.rename_obfuscated,
                },
                &mappings.names,
                &mappings.origins,
            )?;
            diagnostics.extend(mappings.diagnostics.iter().map(ToString::to_string));
            diagnostics.extend(names.diagnostics.iter().cloned());
            let metadata = SessionMetadata {
                classes: (0..program.own_count())
                    .map(|id| SessionClass {
                        name: program.class(id).name.clone(),
                        source_name: names.class_name(id).to_owned(),
                    })
                    .collect(),
                diagnostics,
                api_classes: classes.libraries.len(),
            };
            Ok(Prepared {
                program,
                mappings,
                names,
                metadata,
            })
        });
        match prepared {
            Ok(prepared) => {
                self.prepared = Some(prepared);
                self.options = Some(options);
                Ok(&self.ready()?.metadata)
            }
            Err((classes, error)) => {
                self.classes = classes.own;
                Err(error)
            }
        }
    }

    pub fn decompile(&self, original_name: &str) -> Result<DecompiledClass> {
        let prepared = self.ready()?;
        let id = prepared
            .program
            .id(original_name)
            .filter(|&id| prepared.program.is_own(id))
            .with_context(|| format!("class is not in session: {original_name}"))?;
        decompile_class(
            &prepared.program,
            &prepared.names,
            id,
            &JavaOptions {
                mappings: Some(&prepared.mappings),
                rename_comments: self
                    .options
                    .as_ref()
                    .expect("prepared options")
                    .rename_comments,
            },
        )
    }

    pub fn mapping_tiny(&self) -> Result<String> {
        let prepared = self.ready()?;
        crate::naming::mapping_text(&prepared.program, &prepared.names)
    }

    pub fn domain_sources(&self) -> Result<Vec<DomainSource>> {
        let prepared = self.ready()?;
        Ok(
            domain_sources(&prepared.mappings, &prepared.program, &prepared.names)?
                .into_iter()
                .map(|(name, source)| DomainSource { name, source })
                .collect(),
        )
    }

    fn ready(&self) -> Result<&Prepared<'_>> {
        self.prepared
            .as_ref()
            .map(PreparedCell::borrow_dependent)
            .context("session is not prepared; call prepare after changing inputs or mappings")
    }

    fn invalidate(&mut self) {
        if let Some(prepared) = self.prepared.take() {
            self.classes = prepared.into_owner().own;
        }
        self.options = None;
    }
}
