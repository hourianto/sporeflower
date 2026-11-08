//! Deterministic class input. Archive entry paths never become output paths.
use crate::{Result, classfile::ClassFile};
use anyhow::bail;
#[cfg(not(target_arch = "wasm32"))]
use anyhow::{Context, ensure};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
#[cfg(not(target_arch = "wasm32"))]
use std::{collections::BTreeSet, fs, io::Read, sync::Arc};

/// Classes retain their original binary identities, regardless of ZIP entry spelling.
/// Parsed classes are immutable while attached to their original bytes.
pub struct Input {
    classes: Vec<ClassFile>,
    #[cfg(not(target_arch = "wasm32"))]
    native: Option<NativeInput>,
    pub manifest: BTreeMap<String, String>,
    pub warnings: Vec<String>,
}

/// Original bytes retained only when native output is requested.
#[derive(Default)]
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct NativeInput {
    pub classes: BTreeMap<String, Arc<[u8]>>,
    pub resources: BTreeMap<String, Arc<[u8]>>,
}

impl Input {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read(path: &Path) -> Result<Self> {
        Self::read_inner(path, false)
    }

    /// Read classes and resources once, retaining the bytes for native remapping.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_for_remap(path: &Path) -> Result<Self> {
        Self::read_inner(path, true)
    }

    pub fn classes(&self) -> &[ClassFile] {
        &self.classes
    }

    pub fn analysis_scope(&self) -> crate::program::AnalysisScope {
        analysis_scope(&self.classes, &self.manifest)
    }

    /// Detach models for callers that need to edit them. Original bytes are dropped.
    pub fn into_classes(self) -> Vec<ClassFile> {
        self.classes
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn native(&self) -> Option<&NativeInput> {
        self.native.as_ref()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn read_inner(path: &Path, retain: bool) -> Result<Self> {
        let mut input = Self {
            classes: Vec::new(),
            native: retain.then(NativeInput::default),
            manifest: BTreeMap::new(),
            warnings: Vec::new(),
        };
        if path.is_dir() {
            let mut paths = Vec::new();
            collect_files(path, path, &mut paths, retain)?;
            paths.sort();
            for file in paths {
                let relative = file.strip_prefix(path)?;
                let name = if retain {
                    relative.to_str().context("non-UTF-8 resource path")?.into()
                } else {
                    relative.to_string_lossy()
                }
                .replace(std::path::MAIN_SEPARATOR, "/");
                if retain {
                    safe_entry_name(&name)?;
                }
                let bytes = fs::read(&file).with_context(|| format!("read {}", file.display()))?;
                input.entry(bytes, &name)?;
            }
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("class"))
        {
            input.push(fs::read(path)?, &path.display().to_string())?;
        } else {
            let file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
            let mut archive = zip::ZipArchive::new(file).context("read JAR directory")?;
            let mut entries = BTreeSet::new();
            for i in 0..archive.len() {
                let mut entry = archive.by_index(i)?;
                let name = entry.name()?.into_owned();
                if entry.is_dir() {
                    continue;
                }
                if retain {
                    safe_entry_name(&name)?;
                    ensure!(
                        entries.insert(name.clone()),
                        "duplicate input JAR entry {name}"
                    );
                }
                if name.ends_with(".class") {
                    // A valid JVM method has at most 64 KiB of bytecode. This generous
                    // class limit still prevents accidental unbounded ZIP expansion.
                    ensure!(
                        entry.size() <= 64 * 1024 * 1024,
                        "class entry {name} exceeds 64 MiB"
                    );
                } else if name.eq_ignore_ascii_case("META-INF/MANIFEST.MF") {
                    ensure!(entry.size() <= 1024 * 1024, "manifest exceeds 1 MiB");
                } else if !retain {
                    continue;
                }
                let mut bytes = Vec::new();
                entry
                    .read_to_end(&mut bytes)
                    .with_context(|| format!("read {name}"))?;
                input.entry(bytes, &name)?;
            }
        }
        input.classes.sort_by(|a, b| a.name.cmp(&b.name));
        for pair in input.classes.windows(2) {
            ensure!(
                pair[0].name != pair[1].name,
                "duplicate class identity {}",
                pair[0].name
            );
        }
        ensure!(
            !input.classes.is_empty(),
            "no classes in {}",
            path.display()
        );
        Ok(input)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn entry(&mut self, bytes: Vec<u8>, name: &str) -> Result<()> {
        if name.ends_with(".class") {
            if bytes.starts_with(b"\xca\xfe\xba\xbe") {
                return self.push(bytes, name);
            }
            // Some MIDlets deliberately use .class names for icons and resources.
            self.warnings
                .push(format!("ignored non-class resource {name}"));
        } else if name.eq_ignore_ascii_case("META-INF/MANIFEST.MF") {
            let (manifest, malformed) = parse_manifest_bytes(&bytes);
            self.manifest = manifest;
            if malformed {
                self.warnings.push(
                    "manifest contains non-UTF-8 text; invalid metadata bytes were replaced".into(),
                );
            }
        }
        if let Some(native) = &mut self.native {
            native.resources.insert(name.to_owned(), bytes.into());
        }
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn push(&mut self, bytes: Vec<u8>, entry: &str) -> Result<()> {
        let class = ClassFile::parse(&bytes).with_context(|| format!("parse class {entry}"))?;
        for warning in &class.warnings {
            self.warnings.push(format!("{}: {warning}", class.name));
        }
        if let Some(native) = &mut self.native {
            native.classes.insert(class.name.clone(), bytes.into());
        }
        self.classes.push(class);
        Ok(())
    }
}

/// An application manifest declares the intended unit of analysis. This is an
/// assumption that application callers, writers and subclasses were supplied,
/// not proof against arbitrary omitted code. Loose classes/library archives
/// remain partial; an entry naming an absent class cannot close the boundary.
pub(crate) fn analysis_scope(
    classes: &[ClassFile],
    manifest: &BTreeMap<String, String>,
) -> crate::program::AnalysisScope {
    let entries: Vec<_> = manifest
        .iter()
        .filter_map(|(key, value)| {
            if key == "main-class" || key == "midlet-class" {
                Some(value.trim().replace('.', "/"))
            } else if key
                .strip_prefix("midlet-")
                .is_some_and(|suffix| suffix.parse::<u32>().is_ok_and(|number| number > 0))
            {
                // A malformed additional entry must not be silently ignored.
                Some(
                    value
                        .rsplit_once(',')
                        .map_or_else(String::new, |(_, class)| class.trim().replace('.', "/")),
                )
            } else {
                None
            }
        })
        .collect();
    if !entries.is_empty()
        && entries
            .iter()
            .all(|entry| classes.iter().any(|class| &class.name == entry))
    {
        crate::program::AnalysisScope::Application
    } else {
        crate::program::AnalysisScope::Partial
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn collect_files(
    root: &Path,
    directory: &Path,
    out: &mut Vec<PathBuf>,
    resources: bool,
) -> Result<()> {
    for entry in fs::read_dir(directory).with_context(|| format!("list {}", directory.display()))? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_files(root, &entry.path(), out, resources)?;
        } else if kind.is_file() {
            if resources
                || entry.path().extension().is_some_and(|x| x == "class")
                || entry
                    .path()
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/")
                    .eq_ignore_ascii_case("META-INF/MANIFEST.MF")
            {
                out.push(entry.path());
            }
        } else if resources {
            bail!(
                "unsupported symlink or special input entry {}",
                entry.path().display()
            );
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn safe_entry_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && !name.split('/').any(|part| part.is_empty()
                || part == "."
                || part == ".."
                || part.contains(['\\', '\0', ':'])),
        "unsafe JAR entry {name:?}"
    );
    Ok(())
}

pub fn parse_manifest(text: &str) -> BTreeMap<String, String> {
    parse_manifest_bytes(text.as_bytes()).0
}

/// Manifest folding counts bytes, so unfold before decoding UTF-8. Invalid
/// descriptive metadata must not prevent valid classes from being decompiled.
fn parse_manifest_bytes(bytes: &[u8]) -> (BTreeMap<String, String>, bool) {
    let mut attributes = BTreeMap::<String, Vec<u8>>::new();
    let mut current: Option<String> = None;
    let mut malformed = false;
    for line in bytes.split(|&byte| byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            break;
        } // Only the main attributes apply to the MIDlet.
        if let Some(rest) = line.strip_prefix(b" ") {
            if let Some(key) = &current {
                attributes
                    .entry(key.clone())
                    .or_default()
                    .extend_from_slice(rest);
            }
        } else if let Some(colon) = line.iter().position(|&byte| byte == b':') {
            let key = String::from_utf8_lossy(&line[..colon]);
            malformed |= matches!(key, std::borrow::Cow::Owned(_));
            let key = key.to_ascii_lowercase();
            let value = line[colon + 1..].trim_ascii_start();
            attributes.insert(key.clone(), value.to_vec());
            current = Some(key);
        } else {
            current = None;
        }
    }
    let attributes = attributes
        .into_iter()
        .map(|(key, bytes)| {
            let value = String::from_utf8_lossy(&bytes);
            malformed |= matches!(value, std::borrow::Cow::Owned(_));
            (key, value.into_owned())
        })
        .collect();
    (attributes, malformed)
}

/// Validate generated internal names independently of a naming policy.
pub fn source_path(root: &Path, binary_name: &str) -> Result<PathBuf> {
    if binary_name.is_empty()
        || binary_name
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == ".." || s.contains(['\\', '\0', ':']))
    {
        bail!("invalid output class name {binary_name:?}");
    }
    Ok(root.join(format!("{binary_name}.java")))
}
