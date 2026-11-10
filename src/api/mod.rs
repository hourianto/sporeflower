//! Embedded CLDC, MIDP, JSR and vendor API declarations.
//!
//! Selection uses original JVM descriptors and invocation kinds. A selected class
//! always comes from one provider: incompatible SDK revisions are never combined
//! into a fictitious API. Separate bytecode-inferred compiler placeholders are
//! export-only and never participate in semantic analysis. The exported JAR is
//! a compile-time library, not an emulator.

mod inferred;
mod requirements;
mod schema;
mod selection;
mod writer;

use crate::hash::{HashMap, HashSet};
use crate::{Result, classfile::ClassFile};
use anyhow::{Context, ensure};
use std::{collections::BTreeMap, sync::OnceLock};
#[cfg(not(target_arch = "wasm32"))]
use std::{io::Write, path::Path};

static PACKED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/j2me-api.bin"));

#[derive(Clone, Debug, Default)]
pub struct ApiOptions {
    /// Manifest MicroEdition-Configuration, normally `CLDC-1.0` or `CLDC-1.1`.
    pub configuration: Option<String>,
    /// Export the entire selected API surface rather than its dependency closure.
    pub include_all: bool,
    /// Additional real API declarations needed by mapping constant providers.
    /// Missing catalog owners do not manufacture analysis classes or stubs.
    pub additional_roots: Vec<String>,
}

/// Immutable, shared API metadata. Class bytes and names borrow the embedded pack.
pub struct ApiCatalog {
    libraries: Vec<Library>,
    providers: BTreeMap<&'static str, Vec<Candidate>>,
    core_owners: HashSet<&'static str>,
    boot_providers: Vec<usize>,
}

struct Library {
    name: &'static str,
    fallback: bool,
    classes: Vec<Definition>,
    by_name: HashMap<&'static str, usize>,
}

struct Definition {
    name: &'static str,
    access: u16,
    bytes: &'static [u8],
    parents: Vec<&'static str>,
    members: HashSet<(&'static str, &'static str, bool, bool)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Candidate {
    library: usize,
    class: usize,
}

/// Provenanced declarations for analysis, their exact corresponding compile
/// bytes, and separate export-only placeholders for unknown external classes.
pub struct ApiSelection {
    pub classes: Vec<ClassFile>,
    pub providers: BTreeMap<String, String>,
    pub warnings: Vec<String>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    bytes: BTreeMap<&'static str, &'static [u8]>,
    // Compiler placeholders must never join `classes`: hierarchy and exception
    // analysis may only consume declarations with actual API provenance.
    inferred: BTreeMap<String, inferred::CompileClass>,
}

impl ApiCatalog {
    /// A canonical platform body whose dispatch cannot select an override.
    /// Available even when callers deliberately omit bundled declarations.
    pub(crate) fn final_boolean_return(&self, method: &crate::ir::Member) -> bool {
        matches!(
            method.owner.as_str(),
            "java/lang/String" | "java/io/DataInputStream"
        ) && self.canonical_boolean_return(method)
    }

    /// These platform implementations produce predicates, rather than copying
    /// a declared Boolean slot. Match real catalog declarations as well as the
    /// semantic contract; unknown external Z methods receive no certificate.
    pub(crate) fn canonical_boolean_return(&self, method: &crate::ir::Member) -> bool {
        let predicate = match method.owner.as_str() {
            "java/lang/Object"
            | "java/lang/Boolean"
            | "java/lang/Byte"
            | "java/lang/Character"
            | "java/lang/Short"
            | "java/lang/Integer"
            | "java/lang/Long"
            | "java/lang/Float"
            | "java/lang/Double"
            | "java/util/Calendar"
            | "java/util/Date"
            | "javax/bluetooth/UUID"
            | "javax/bluetooth/RemoteDevice" => {
                method.name == "equals" && method.descriptor == "(Ljava/lang/Object;)Z"
            }
            "java/lang/String" => matches!(
                method.name.as_str(),
                "equals" | "equalsIgnoreCase" | "startsWith" | "endsWith" | "regionMatches"
            ),
            "java/util/Vector" => matches!(
                method.name.as_str(),
                "contains" | "isEmpty" | "removeElement"
            ),
            "java/util/Hashtable" => {
                matches!(method.name.as_str(), "contains" | "containsKey" | "isEmpty")
            }
            "java/io/DataInputStream" => method.name == "readBoolean",
            _ => false,
        };
        predicate
            && method.descriptor.ends_with(")Z")
            && self
                .providers
                .get(method.owner.as_str())
                .is_some_and(|providers| {
                    providers.iter().any(|&provider| {
                        self.definition(provider).members.contains(&(
                            method.name.as_str(),
                            method.descriptor.as_str(),
                            false,
                            false,
                        ))
                    })
                })
    }

    pub fn bundled() -> Result<&'static Self> {
        static CATALOG: OnceLock<std::result::Result<ApiCatalog, String>> = OnceLock::new();
        match CATALOG.get_or_init(|| Self::read(PACKED).map_err(|error| format!("{error:#}"))) {
            Ok(catalog) => Ok(catalog),
            Err(message) => Err(anyhow::anyhow!(message.clone())),
        }
    }

    pub fn select(&self, project: &[ClassFile], options: &ApiOptions) -> Result<ApiSelection> {
        self.select_with_classpath(project, &[], options)
    }

    /// Explicit library declarations take precedence over bundled and inferred
    /// exports. They are supplied to semantic analysis separately by the caller.
    pub fn select_with_classpath(
        &self,
        project: &[ClassFile],
        classpath: &[ClassFile],
        options: &ApiOptions,
    ) -> Result<ApiSelection> {
        selection::select(self, project, classpath, options)
    }

    pub fn variant_count(&self) -> usize {
        self.libraries.len()
    }
    pub fn class_count(&self) -> usize {
        self.providers.len()
    }

    fn definition(&self, candidate: Candidate) -> &Definition {
        &self.libraries[candidate.library].classes[candidate.class]
    }

    fn read(bytes: &'static [u8]) -> Result<Self> {
        let mut input = Reader { bytes, position: 0 };
        ensure!(
            input.take(8)? == b"SFAPI\0\x01\0",
            "unsupported API declaration pack"
        );
        let library_count = input.count()?;
        let mut libraries = Vec::with_capacity(library_count);
        let mut providers: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for library in 0..library_count {
            let name = input.text()?;
            let fallback = input.byte()? != 0;
            let class_count = input.count()?;
            let mut classes = Vec::with_capacity(class_count);
            let mut by_name = HashMap::with_capacity_and_hasher(class_count, Default::default());
            for class in 0..class_count {
                let name = input.text()?;
                let access = input.u16()?;
                let length = input.count()?;
                let bytes = input.take(length)?;
                let parent_count = input.count()?;
                let mut parents = Vec::with_capacity(parent_count);
                for _ in 0..parent_count {
                    parents.push(input.text()?);
                }
                let member_count = input.count()?;
                let mut members =
                    HashSet::with_capacity_and_hasher(member_count, Default::default());
                for _ in 0..member_count {
                    let name = input.text()?;
                    let descriptor = input.text()?;
                    let flags = input.byte()?;
                    members.insert((name, descriptor, flags & 1 != 0, flags & 2 != 0));
                }
                ensure!(
                    by_name.insert(name, class).is_none(),
                    "duplicate API class {name}"
                );
                classes.push(Definition {
                    name,
                    access,
                    bytes,
                    parents,
                    members,
                });
                providers
                    .entry(name)
                    .or_default()
                    .push(Candidate { library, class });
            }
            libraries.push(Library {
                name,
                fallback,
                classes,
                by_name,
            });
        }
        ensure!(
            input.position == bytes.len(),
            "trailing API declaration data"
        );
        let boot_providers: Vec<_> = providers
            .get("java/lang/Object")
            .into_iter()
            .flatten()
            .map(|candidate| candidate.library)
            .collect();
        let standalone: Vec<_> = boot_providers
            .iter()
            .copied()
            .filter(|&library| {
                !libraries[library]
                    .by_name
                    .contains_key("javax/microedition/midlet/MIDlet")
            })
            .collect();
        let mut core_owners = HashSet::default();
        for &library in if standalone.is_empty() {
            &boot_providers
        } else {
            &standalone
        } {
            core_owners.extend(
                libraries[library]
                    .by_name
                    .keys()
                    .copied()
                    .filter(|name| name.starts_with("java/")),
            );
        }
        Ok(Self {
            libraries,
            providers,
            core_owners,
            boot_providers,
        })
    }
}

impl Definition {
    fn declares(&self, member: &requirements::Member) -> bool {
        let key = (
            member.name.as_str(),
            member.descriptor.as_str(),
            member.field,
        );
        match member.is_static {
            Some(is_static) => self.members.contains(&(key.0, key.1, key.2, is_static)),
            None => {
                self.members.contains(&(key.0, key.1, key.2, false))
                    || self.members.contains(&(key.0, key.1, key.2, true))
            }
        }
    }
}

impl ApiSelection {
    pub fn inferred_class_count(&self) -> usize {
        self.inferred.len()
    }

    /// Write deterministic compile-only declarations. No SDK implementation is copied.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn write_jar(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut pending = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("stage API stubs {}", path.display()))?;
        let mut archive = zip::ZipWriter::new(&mut pending);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o644);
        archive.start_file("META-INF/MANIFEST.MF", options)?;
        write!(
            archive,
            "Manifest-Version: 1.0\r\nJ2ME-Stub-Kind: compile-only\r\nJ2ME-Stub-Fallback: {}\r\nJ2ME-Inferred-Stub-Count: {}\r\n\r\n",
            !self.inferred.is_empty(),
            self.inferred.len()
        )?;
        for (name, bytes) in &self.bytes {
            archive.start_file(format!("{name}.class"), options)?;
            archive.write_all(bytes)?;
        }
        for (name, class) in &self.inferred {
            archive.start_file(format!("{name}.class"), options)?;
            archive.write_all(&class.bytes)?;
        }
        archive.start_file("META-INF/sporeflower-api.tsv", options)?;
        archive.write_all(b"class\tprovider\n")?;
        for (name, provider) in &self.providers {
            writeln!(archive, "{name}\t{provider}")?;
        }
        if !self.inferred.is_empty() {
            archive.start_file("META-INF/sporeflower-inferred-api.json", options)?;
            serde_json::to_writer_pretty(&mut archive, &self.inferred)?;
            archive.write_all(b"\n")?;
        }
        archive.finish()?;
        pending
            .persist(path)
            .with_context(|| format!("replace API stubs {}", path.display()))?;
        Ok(())
    }
}

struct Reader {
    bytes: &'static [u8],
    position: usize,
}

impl Reader {
    fn take(&mut self, length: usize) -> Result<&'static [u8]> {
        let end = self
            .position
            .checked_add(length)
            .context("API pack offset overflow")?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .context("truncated API declaration pack")?;
        self.position = end;
        Ok(bytes)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into()?))
    }
    fn count(&mut self) -> Result<usize> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into()?) as usize)
    }
    fn text(&mut self) -> Result<&'static str> {
        let length = self.count()?;
        Ok(std::str::from_utf8(self.take(length)?)?)
    }
}

#[cfg(test)]
mod publication_tests {
    use super::*;

    #[test]
    fn failed_export_preserves_previous_api_jar() {
        let work = tempfile::tempdir().unwrap();
        let path = work.path().join("api.jar");
        std::fs::write(&path, b"previous API").unwrap();
        // ZIP filenames are limited to u16 bytes; fail after writing the manifest.
        let invalid_name: &'static str = Box::leak("x".repeat(65536).into_boxed_str());
        let selected = ApiSelection {
            classes: vec![],
            providers: BTreeMap::new(),
            warnings: vec![],
            bytes: [(invalid_name, &b"class"[..])].into(),
            inferred: BTreeMap::new(),
        };
        assert!(selected.write_jar(&path).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"previous API");
        assert_eq!(std::fs::read_dir(work.path()).unwrap().count(), 1);
    }
}
