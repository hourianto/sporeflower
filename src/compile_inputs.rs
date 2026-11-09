//! Resolved inputs travel with generated sources; compilation does not repeat
//! mapping or API selection. Classpath entries always use compiler search order.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompileInputs {
    pub api: ApiInput,
    pub classpath: Classpath,
    pub bytecode_major: u16,
}

impl CompileInputs {
    pub const FILE: &str = ".sporeflower-compile.json";

    pub fn read(sources: &Path) -> Result<Option<Self>> {
        let sources = sources.canonicalize()?;
        let path = sources.join(Self::FILE);
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        let inputs: Self =
            serde_json::from_reader(file).with_context(|| format!("parse {}", path.display()))?;
        Ok(Some(inputs.resolve_from(&sources)))
    }

    pub fn write(self, sources: &Path, project: &Path) -> Result<()> {
        let sources = crate::artifact_plan::resolved(sources)?;
        let inputs = self.for_sources(&sources, project)?;
        crate::output::write_file(
            &sources.join(Self::FILE),
            &serde_json::to_vec_pretty(&inputs)?,
        )
    }

    fn paths_mut(&mut self) -> impl Iterator<Item = &mut PathBuf> {
        let api = match &mut self.api {
            ApiInput::Exported(path) => Some(path),
            _ => None,
        };
        api.into_iter().chain(self.classpath.0.iter_mut())
    }

    /// Project-owned inputs move with their source tree; external SDKs keep
    /// their absolute identities. Both roots are canonical filesystem paths.
    pub fn for_sources(mut self, sources: &Path, project: &Path) -> Result<Self> {
        let project = crate::artifact_plan::resolved(project)?;
        if let Ok(suffix) = sources.strip_prefix(&project) {
            let parent: PathBuf = suffix.components().map(|_| "..").collect();
            for path in self.paths_mut() {
                if let Ok(relative) = path.strip_prefix(&project) {
                    let relative = parent.join(relative);
                    // Both platforms accept '/' as a separator. Backslashes
                    // saved by Windows would be filename characters on Unix.
                    #[cfg(windows)]
                    let relative = PathBuf::from(
                        relative
                            .to_str()
                            .context("saved compilation inputs require UTF-8 paths")?
                            .replace('\\', "/"),
                    );
                    *path = relative;
                }
            }
        }
        Ok(self)
    }

    pub fn resolve_from(mut self, sources: &Path) -> Self {
        for path in self.paths_mut() {
            *path = sources.join(&*path);
        }
        self
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiInput {
    Disabled,
    NotExported,
    Exported(PathBuf),
}

#[derive(Default, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Classpath(Vec<PathBuf>);

impl Classpath {
    /// Configured paths precede CLI additions; the last declaration wins.
    /// Store the reverse order so analysis and Java compilers both search first.
    pub fn resolve(paths: impl IntoIterator<Item = PathBuf>) -> Result<Self> {
        Self::default().with_overrides(paths)
    }

    pub fn with_overrides(self, paths: impl IntoIterator<Item = PathBuf>) -> Result<Self> {
        let mut overrides: Vec<_> = paths.into_iter().collect();
        overrides.reverse();
        let mut seen = HashSet::new();
        let mut resolved = Vec::new();
        for path in overrides.into_iter().chain(self.0) {
            let path = path
                .canonicalize()
                .with_context(|| format!("resolve classpath {}", path.display()))?;
            if seen.insert(path.clone()) {
                resolved.push(path);
            }
        }
        Ok(Self(resolved))
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.0
    }
}
