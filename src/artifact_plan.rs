//! Check output collisions and input aliases before generating files.
use anyhow::{Context, Result};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Component, Path, PathBuf},
};

#[derive(Default)]
pub struct ArtifactPlan {
    outputs: BTreeMap<PathBuf, String>,
    identities: BTreeMap<(u64, u64), PathBuf>,
    cleared_directory: Option<PathBuf>,
}

impl ArtifactPlan {
    pub fn for_directory(directory: &Path) -> Result<Self> {
        Ok(Self {
            cleared_directory: Some(resolved(directory)?),
            ..Self::default()
        })
    }

    /// Entries below the generated root disappear before writing. Their old
    /// symlinks, casing, and file kinds do not describe the next generation.
    pub fn resolve_output(&self, path: &Path) -> Result<PathBuf> {
        resolve(path, self.cleared_directory.as_deref())
    }

    pub fn output(&mut self, role: impl Into<String>, path: impl AsRef<Path>) -> Result<()> {
        let role = role.into();
        let path = self.resolve_output(path.as_ref())?;
        if let Some(directory) = &self.cleared_directory {
            anyhow::ensure!(
                !directory.starts_with(&path),
                "output collision: {role} {} conflicts with generated directory {}",
                path.display(),
                directory.display()
            );
        }
        let identity = if self
            .cleared_directory
            .as_ref()
            .is_some_and(|directory| path.starts_with(directory))
        {
            None
        } else {
            file_identity(&path)
        };
        if let Some(other) = self
            .overlapping(&path)
            .or_else(|| identity.and_then(|id| self.identities.get(&id).map(PathBuf::as_path)))
        {
            let other_role = &self.outputs[other];
            anyhow::bail!(
                "output collision: {role} {} conflicts with {other_role} {}",
                path.display(),
                other.display()
            );
        }
        if let Some(identity) = identity {
            self.identities.insert(identity, path.clone());
        }
        self.outputs.insert(path, role);
        Ok(())
    }

    pub fn protect_input(&self, input: &Path) -> Result<()> {
        self.protect_from_clear(input)?;
        let input = resolved(input)?;
        if let Some(output) = self.overlapping(&input).or_else(|| {
            file_identity(&input).and_then(|id| self.identities.get(&id).map(PathBuf::as_path))
        }) {
            let role = &self.outputs[output];
            anyhow::bail!(
                "output collision: {role} {} conflicts with input {}",
                output.display(),
                input.display()
            );
        }
        Ok(())
    }

    /// Directory inputs read only their selected files. Source output may live
    /// beneath a class directory without becoming a binary input itself.
    pub fn protect_directory(&self, input: &Path, extension: &str, recursive: bool) -> Result<()> {
        self.protect_from_clear(input)?;
        if input.is_dir() {
            let mut directories = vec![input.to_owned()];
            while let Some(directory) = directories.pop() {
                for entry in fs::read_dir(directory)? {
                    let entry = entry?;
                    let kind = entry.file_type()?;
                    if recursive && kind.is_dir() {
                        directories.push(entry.path());
                    } else if entry.path().extension().is_some_and(|ext| ext == extension)
                        && (kind.is_file() || !recursive && entry.path().is_file())
                    {
                        self.protect_input(&entry.path())?;
                    }
                }
            }
        } else {
            self.protect_input(input)?;
        }
        Ok(())
    }

    fn protect_from_clear(&self, input: &Path) -> Result<()> {
        if let Some(directory) = &self.cleared_directory {
            let input = self.resolve_output(input)?;
            anyhow::ensure!(
                !input.starts_with(directory),
                "output collision: generated directory {} would remove input {}",
                directory.display(),
                input.display()
            );
        }
        Ok(())
    }

    fn overlapping(&self, path: &Path) -> Option<&Path> {
        for ancestor in path.ancestors() {
            if let Some((key, _)) = self.outputs.get_key_value(ancestor) {
                return Some(key);
            }
        }
        self.outputs
            .range(path.to_path_buf()..)
            .next()
            .filter(|(candidate, _)| candidate.starts_with(path))
            .map(|(candidate, _)| candidate.as_path())
    }
}

fn file_identity(path: &Path) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(metadata) = fs::metadata(path) {
            return Some((metadata.dev(), metadata.ino()));
        }
    }
    None
}

/// Resolve aliases in existing prefixes without creating an output directory.
/// Resolve `..` after its preceding component, preserving symlink semantics.
pub fn resolved(path: &Path) -> Result<PathBuf> {
    resolve(path, None)
}

fn resolve(path: &Path, cleared_directory: Option<&Path>) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut result = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(_) => {
                // A Windows prefix such as `\\?\C:` needs its root separator
                // before it can be opened. Resolving `C:` alone would instead
                // use that drive's current directory.
                result.push(component.as_os_str());
                continue;
            }
            Component::CurDir => continue,
            Component::ParentDir => {
                result.pop();
            }
            _ => result.push(component.as_os_str()),
        }
        if cleared_directory.is_some_and(|directory| result.starts_with(directory)) {
            continue;
        }
        match fs::canonicalize(&result) {
            Ok(path) => result = path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("resolve {}", result.display()));
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_paths_and_missing_descendants_resolve_consistently() {
        let work = tempfile::tempdir().unwrap();
        let canonical = work.path().canonicalize().unwrap();
        // On Windows canonicalize produces a verbatim path (`\\?\C:\...`).
        // Project commands pass these paths back through the artifact planner.
        for root in [work.path(), canonical.as_path()] {
            assert_eq!(resolved(root).unwrap(), canonical);
            assert_eq!(
                resolved(&root.join("missing/output")).unwrap(),
                canonical.join("missing/output")
            );
            let mut plan = ArtifactPlan::for_directory(&root.join("generated")).unwrap();
            plan.output("source", root.join("generated/Example.java"))
                .unwrap();
            plan.protect_input(&root.join("input.jar")).unwrap();
            assert!(
                plan.protect_input(&root.join("generated/input.jar"))
                    .is_err()
            );
        }
        assert_eq!(fs::read_dir(work.path()).unwrap().count(), 0);
    }

    #[test]
    fn aliases_and_ancestor_roles_are_rejected_without_creating_paths() {
        let work = tempfile::tempdir().unwrap();
        let mut plan = ArtifactPlan::default();
        plan.output("manifest", work.path().join("output/manifest"))
            .unwrap();
        assert!(
            plan.output("summary", work.path().join("output/./manifest"))
                .is_err()
        );
        assert!(plan.output("API", work.path().join("output")).is_err());
        assert!(!work.path().join("output").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_hardlink_input_aliases_are_rejected() {
        use std::os::unix::fs::symlink;
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join("input.jar");
        fs::write(&input, b"input").unwrap();
        for (name, hard) in [("hard", true), ("symbolic", false)] {
            let alias = work.path().join(name);
            if hard {
                fs::hard_link(&input, &alias).unwrap();
            } else {
                symlink(&input, &alias).unwrap();
            }
            let mut plan = ArtifactPlan::default();
            plan.output("API", alias).unwrap();
            assert!(plan.protect_input(&input).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn directory_read_set_allows_sources_but_protects_binary_aliases() {
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join("classes");
        fs::create_dir(&input).unwrap();
        let class = input.join("Example.class");
        fs::write(&class, b"class").unwrap();
        let mut sources = ArtifactPlan::default();
        sources
            .output("source", input.join("generated/Example.java"))
            .unwrap();
        sources.protect_directory(&input, "class", true).unwrap();
        let alias = work.path().join("summary.json");
        fs::hard_link(&class, &alias).unwrap();
        sources.output("summary", alias).unwrap();
        assert!(sources.protect_directory(&input, "class", true).is_err());
    }

    #[test]
    fn clearing_protects_input_roots_but_allows_auxiliary_outputs() {
        let work = tempfile::tempdir().unwrap();
        let root = work.path().join("generated");
        let mut plan = ArtifactPlan::for_directory(&root).unwrap();
        plan.output("API", root.join("api.jar")).unwrap();
        plan.output("summary", root.join("result.json")).unwrap();
        assert!(plan.protect_input(&root.join("input.jar")).is_err());
        assert!(plan.protect_directory(&root, "class", true).is_err());
        assert!(plan.output("ancestor", work.path()).is_err());
        assert!(!root.exists());
    }
}
