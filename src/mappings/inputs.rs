//! Read authored inputs once, discover declaration dependencies, then bind them
//! after API selection has supplied real JVM identities for constant providers.
use super::{
    MappingPlan, MappingSource, builtins,
    parser::{self, Annotation, ParsedSource, Type, Value},
    resolve::{self, collect_type},
};
use crate::{
    Result,
    classfile::{ClassFile, Constant, JvmType, MethodDescriptor},
    program::Program,
};
#[cfg(not(target_arch = "wasm32"))]
use anyhow::{Context, ensure};
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::OnceLock,
};

#[derive(Clone, Debug, Default)]
pub struct MappingInputs {
    sources: Vec<ParsedSource>,
}

impl MappingInputs {
    /// Parse authored mapping text without accessing the filesystem.
    pub fn from_sources(sources: &[MappingSource]) -> Result<Self> {
        Ok(Self {
            sources: parser::parse_all(sources)?,
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn read(paths: &[PathBuf]) -> Result<Self> {
        let mut files = Vec::new();
        for path in paths {
            if path.is_dir() {
                let mut entries = std::fs::read_dir(path)
                    .with_context(|| format!("read mappings {}", path.display()))?
                    .map(|entry| entry.map(|entry| entry.path()))
                    .collect::<std::io::Result<Vec<_>>>()?;
                entries.retain(|entry| {
                    entry.extension().is_some_and(|ext| ext == "map") && entry.is_file()
                });
                entries.sort();
                files.extend(entries);
            } else {
                ensure!(
                    path.is_file(),
                    "mapping input does not exist: {}",
                    path.display()
                );
                files.push(path.clone());
            }
        }
        let mut sources = Vec::new();
        for path in files {
            let source = MappingSource::new(
                path.display().to_string(),
                std::fs::read_to_string(&path)
                    .with_context(|| format!("read mapping {}", path.display()))?,
            );
            sources.push(source);
        }
        Self::from_sources(&sources)
    }

    pub fn resolve(&self, program: &Program<'_>, include_builtins: bool) -> Result<MappingPlan> {
        let mut sources = if include_builtins {
            parser::parse_all(&builtins::sources(program))?
        } else {
            Vec::new()
        };
        sources.extend(self.sources.iter().cloned());
        resolve::resolve(program, &sources)
    }

    /// These are declaration requests, not claims that a runtime class exists.
    /// API selection retains only actual providers; strict binding happens later.
    pub fn required_api_roots(
        &self,
        project: &[ClassFile],
        include_builtins: bool,
    ) -> Result<Vec<String>> {
        let packs = if include_builtins {
            builtin_packs()?
        } else {
            &[]
        };
        let mut providers = HashMap::<String, usize>::new();
        for (index, pack) in packs.iter().enumerate() {
            for declaration in &pack.source.declarations {
                if resolve::domain_kind(&declaration.annotations)?.is_some() {
                    providers.insert(
                        resolve::qualified(&pack.source.package, &declaration.name),
                        index,
                    );
                }
            }
        }
        let authored: HashSet<_> = self
            .sources
            .iter()
            .flat_map(|source| {
                source
                    .declarations
                    .iter()
                    .map(|declaration| resolve::qualified(&source.package, &declaration.name))
            })
            .collect();
        let authored_simple: HashSet<_> = authored
            .iter()
            .map(|name| name.rsplit('.').next().unwrap())
            .collect();
        let mut roots = BTreeSet::new();
        let mut requested_packs = HashSet::new();
        for source in &self.sources {
            for import in &source.imports {
                if import.ends_with(".*") || authored.contains(import) {
                    continue;
                }
                if let Some(index) = providers.get(import) {
                    requested_packs.insert(*index);
                } else {
                    roots.insert(import.replace('.', "/"));
                }
            }
            for declaration in &source.declarations {
                if resolve::has(&declaration.annotations, "External") {
                    roots.insert(
                        resolve::qualified(&source.package, &declaration.name).replace('.', "/"),
                    );
                }
                let mut annotations: Vec<_> = declaration.annotations.iter().collect();
                for member in &declaration.members {
                    request_type(source, &member.ty, &authored, &authored_simple, &mut roots);
                    annotations.extend(&member.annotations);
                    for parameter in member.parameters.iter().flatten() {
                        request_type(
                            source,
                            &parameter.ty,
                            &authored,
                            &authored_simple,
                            &mut roots,
                        );
                        annotations.extend(&parameter.annotations);
                    }
                }
                for annotation in annotations {
                    for name in annotation_classes(annotation) {
                        for (domain, index) in &providers {
                            if domain_matches(source, name, domain) && !authored.contains(domain) {
                                requested_packs.insert(*index);
                            }
                        }
                    }
                }
            }
        }
        if include_builtins {
            let own: HashSet<_> = project.iter().map(|class| class.name.as_str()).collect();
            let mut references = HashSet::new();
            for class in project {
                references.extend(class.super_name.iter().cloned());
                references.extend(class.interfaces.iter().cloned());
                for (index, constant) in class.constant_pool.entries.iter().enumerate() {
                    if matches!(constant, Constant::Class(_))
                        && let Ok(owner) = class.constant_pool.class_name(index as u16)
                    {
                        if owner.starts_with('[') {
                            if let Ok(ty) = JvmType::parse(owner) {
                                collect_type(&ty, &mut references);
                            }
                        } else {
                            references.insert(owner.into());
                        }
                    }
                }
                for field in &class.fields {
                    collect_type(&JvmType::parse(&field.descriptor)?, &mut references);
                }
                for method in &class.methods {
                    let descriptor = MethodDescriptor::parse(&method.descriptor)?;
                    collect_type(&descriptor.return_type, &mut references);
                    for parameter in descriptor.parameters {
                        collect_type(&parameter, &mut references);
                    }
                }
            }
            references.retain(|name| !own.contains(name.as_str()));
            references.extend(roots.iter().cloned());
            loop {
                let mut changed = false;
                for (index, pack) in packs.iter().enumerate() {
                    if requested_packs.contains(&index)
                        || pack
                            .external_owners
                            .iter()
                            .any(|owner| references.contains(owner))
                    {
                        for owner in &pack.required {
                            changed |= references.insert(owner.clone());
                            roots.insert(owner.clone());
                        }
                    }
                }
                if !changed {
                    break;
                }
            }
        }
        Ok(roots.into_iter().collect())
    }
}

fn request_type(
    source: &ParsedSource,
    ty: &Type,
    authored: &HashSet<String>,
    authored_simple: &HashSet<&str>,
    roots: &mut BTreeSet<String>,
) {
    let name = &ty.name;
    if [
        "void", "boolean", "byte", "short", "char", "int", "long", "float", "double",
    ]
    .contains(&name.as_str())
        || authored.contains(name)
        || !name.contains('.') && authored_simple.contains(name.as_str())
    {
        return;
    }
    if name.contains('.') {
        roots.insert(name.replace('.', "/"));
        return;
    }
    if let Some(import) = source.explicit_import(name) {
        roots.insert(import.replace('.', "/"));
        return;
    }
    roots.insert(resolve::qualified(&source.package, name).replace('.', "/"));
    roots.insert(format!("java/lang/{name}"));
    for import in &source.imports {
        if let Some(package) = import.strip_suffix(".*") {
            roots.insert(resolve::qualified(package, name).replace('.', "/"));
        }
    }
}

fn annotation_classes(annotation: &Annotation) -> Vec<&str> {
    fn collect<'a>(value: &'a Value, output: &mut Vec<&'a str>) {
        match value {
            Value::Class(name) => output.push(name),
            Value::Array(values) => {
                for value in values {
                    collect(value, output);
                }
            }
            _ => {}
        }
    }
    let mut names = Vec::new();
    for value in annotation.args.values() {
        collect(value, &mut names);
    }
    names
}

fn domain_matches(source: &ParsedSource, name: &str, domain: &str) -> bool {
    if name.contains('.') {
        return name == domain;
    }
    if let Some(import) = source.explicit_import(name) {
        return import == domain;
    }
    // Resolution later rejects ambiguity; bringing candidate providers into the
    // classpath here must not choose a meaning prematurely.
    domain.rsplit('.').next() == Some(name)
}

struct BuiltinPack {
    source: ParsedSource,
    required: Vec<String>,
    external_owners: Vec<String>,
}
fn builtin_packs() -> Result<&'static [BuiltinPack]> {
    static PARSED: OnceLock<std::result::Result<Vec<BuiltinPack>, String>> = OnceLock::new();
    match PARSED.get_or_init(|| {
        builtins::PACKS
            .iter()
            .map(|(name, required, text)| {
                let source = parser::parse(&MappingSource {
                    path: format!("builtin-mappings/{name}"),
                    source: (*text).into(),
                    builtin: true,
                })?;
                let external_owners = source
                    .declarations
                    .iter()
                    .filter(|declaration| resolve::has(&declaration.annotations, "External"))
                    .map(|declaration| {
                        resolve::qualified(&source.package, &declaration.name).replace('.', "/")
                    })
                    .collect();
                Ok(BuiltinPack {
                    source,
                    required: required.iter().map(|owner| (*owner).to_owned()).collect(),
                    external_owners,
                })
            })
            .collect::<Result<Vec<_>>>()
            .map_err(|error| format!("{error:#}"))
    }) {
        Ok(packs) => Ok(packs),
        Err(error) => anyhow::bail!("{error}"),
    }
}
