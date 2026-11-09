//! Project commands keep authored inputs separate from reproducible artifacts.
use crate::{
    cli::{DecompileArgs, default_threads},
    decompile, output,
};
use anyhow::{Context, Result, ensure};
use clap::Args;
use serde::Deserialize;
use sporeflower::input::Input;
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Args)]
pub struct ProjectLocation {
    /// Project directory containing j2me.toml.
    #[arg(long, default_value = ".")]
    pub project: PathBuf,
}

#[derive(Args)]
pub struct InitArgs {
    #[command(flatten)]
    location: ProjectLocation,
    /// Source JAR, copied into the project.
    #[arg(long)]
    jar: PathBuf,
    /// Destination JAR filename (the .jar suffix is optional).
    #[arg(long = "as", short = 'n')]
    name: Option<String>,
    /// Replace the project's JAR, configuration and guide; authored mappings stay intact.
    #[arg(long)]
    force: bool,
    /// Omit original-name comments on renamed declarations.
    #[arg(long)]
    no_rename_comments: bool,
    /// Maximum simultaneous workers. Defaults to min(CPU count, 8).
    #[arg(short = 'j', long, default_value_t = default_threads())]
    threads: usize,
}

#[derive(Args)]
pub struct RemapArgs {
    #[command(flatten)]
    location: ProjectLocation,
    /// Additional library JARs or class directories; later entries override project libraries.
    #[arg(short = 'c', long = "classpath")]
    classpath: Vec<PathBuf>,
    /// Maximum simultaneous workers. Defaults to min(CPU count, 8).
    #[arg(short = 'j', long, default_value_t = default_threads())]
    threads: usize,
    /// Additional authored .map files or directories.
    #[arg(long)]
    mappings: Vec<PathBuf>,
    /// Replace short obfuscated class names; illegal names are always repaired.
    #[arg(long)]
    rename_obfuscated: bool,
    /// Omit original-name comments on renamed declarations.
    #[arg(long)]
    no_rename_comments: bool,
    /// Disable bundled API meanings; authored contracts remain active.
    #[arg(long, alias = "no-semantic-mappings")]
    no_builtin_mappings: bool,
    /// Ignore authored and bundled mappings, repairing short class names.
    #[arg(long)]
    raw: bool,
    /// Omit completion and output summaries; diagnostics are still printed.
    #[arg(short, long)]
    quiet: bool,
}

#[derive(Default, Deserialize)]
pub struct BytecodeConfig {
    pub major: Option<u16>,
    pub javac_source: Option<String>,
    pub javac_target: Option<String>,
}

#[derive(Deserialize)]
struct Config {
    jar: PathBuf,
    rename_comments: Option<bool>,
    #[serde(default)]
    classpath: Vec<PathBuf>,
    #[serde(default)]
    bytecode: BytecodeConfig,
    #[serde(default)]
    compiler: crate::compiler::CompilerConfig,
}

pub struct Project {
    pub root: PathBuf,
    config: Config,
}

impl Project {
    /// Load configuration; commands that need the original JAR read it separately.
    pub fn read(root: &Path) -> Result<Self> {
        let root = root
            .canonicalize()
            .with_context(|| format!("resolve project {}", root.display()))?;
        let path = root.join("j2me.toml");
        let config: Config = toml::from_str(&fs::read_to_string(&path).with_context(|| {
            format!(
                "read {}; initialize with sporeflower init --jar <file>",
                path.display()
            )
        })?)
        .with_context(|| format!("parse {}", path.display()))?;
        ensure!(
            !config.jar.as_os_str().is_empty(),
            "{}: jar is empty",
            path.display()
        );
        Ok(Self { root, config })
    }
    pub fn jar(&self) -> PathBuf {
        self.root.join(&self.config.jar)
    }
    pub fn classpath(&self) -> Vec<PathBuf> {
        self.config
            .classpath
            .iter()
            .map(|p| self.root.join(p))
            .collect()
    }
    pub fn bytecode(&self) -> &BytecodeConfig {
        &self.config.bytecode
    }
    pub fn compiler(&self) -> &crate::compiler::CompilerConfig {
        &self.config.compiler
    }
    pub fn rename_comments(&self) -> bool {
        self.config.rename_comments.unwrap_or(true)
    }
}

pub fn remap(args: RemapArgs) -> Result<bool> {
    let project = Project::read(&args.location.project)?;
    let out = project.root.join("out");
    let result = decompile::run_project(
        DecompileArgs {
            output: Some(project.root.join("decompiled")),
            classpath: args.classpath,
            threads: args.threads,
            stubs: Some(out.join("api-stubs.jar")),
            json: Some(out.join("decompile.json")),
            no_mapping: true,
            rename_obfuscated: args.rename_obfuscated || args.raw,
            no_rename_comments: args.no_rename_comments,
            mappings: if args.raw { Vec::new() } else { args.mappings },
            no_semantic_mappings: args.no_builtin_mappings || args.raw,
            mapping_report: Some(out.clone()),
            quiet: args.quiet,
            ..DecompileArgs::default()
        },
        &project,
        &out,
        args.raw,
    )?;
    if !args.quiet {
        eprintln!(
            "Project outputs: {} and {}",
            out.display(),
            project.root.join("decompiled").display()
        );
    }
    Ok(result)
}

pub fn init(args: InitArgs) -> Result<bool> {
    ensure!(args.threads > 0, "--threads must be positive");
    let input = Input::read(&args.jar)?;
    // Validate the archive itself before creating or replacing project inputs.
    let source = args.jar.canonicalize()?;
    ensure!(source.is_file(), "--jar must be a JAR file");
    resource_entries(&source)?;
    let root = &args.location.project;
    let mut name = args
        .name
        .unwrap_or_else(|| source.file_name().unwrap().to_string_lossy().into_owned());
    ensure!(
        !name.is_empty() && !name.contains(['/', '\\', ':', '\0']) && name != "." && name != "..",
        "--as requires a filename, not a path"
    );
    if !name.to_ascii_lowercase().ends_with(".jar") {
        name.push_str(".jar");
    }
    let destination = root.join(&name);
    let config = root.join("j2me.toml");
    let guidance = root.join("mappings/MAPPINGS.md");
    let same_file = same_file(&source, &destination);
    for path in [&destination, &config, &guidance] {
        if let Ok(metadata) = fs::symlink_metadata(path) {
            ensure!(
                metadata.is_file(),
                "refusing non-regular initialization target {}",
                path.display()
            );
            ensure!(
                args.force || path == &destination && same_file,
                "init would overwrite {}; use --force to replace initialization files",
                path.display()
            );
        }
    }
    // Preserve unrelated configuration keys when explicitly reinitializing.
    let mut document = if config.exists() {
        toml::from_str::<toml::Table>(&fs::read_to_string(&config)?)?
    } else {
        toml::Table::new()
    };
    let class = input
        .classes()
        .iter()
        .max_by_key(|c| (c.major_version, c.minor_version))
        .unwrap();
    let (source_level, target_level) = java_levels(class.major_version);
    document.insert("jar".into(), name.clone().into());
    let profile = document
        .entry("bytecode")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .context("[bytecode] must be a TOML table")?;
    for (key, value) in [
        ("major", class.major_version as i64),
        ("minor", class.minor_version as i64),
        ("class_count", input.classes().len() as i64),
    ] {
        profile.insert(key.into(), value.into());
    }
    profile
        .entry("javac_source")
        .or_insert_with(|| source_level.into());
    profile
        .entry("javac_target")
        .or_insert_with(|| target_level.into());
    let config_text = toml::to_string_pretty(&document)?;
    fs::create_dir_all(root.join("mappings"))?;
    if !same_file {
        fs::copy(&source, &destination)
            .with_context(|| format!("copy {}", destination.display()))?;
    }
    fs::write(&config, config_text)?;
    fs::write(guidance, include_str!("../docs/mappings.md"))?;
    eprintln!("Initialized {}", root.display());
    extract(root)?;
    remap(RemapArgs {
        location: args.location,
        classpath: vec![],
        threads: args.threads,
        mappings: vec![],
        rename_obfuscated: false,
        no_rename_comments: args.no_rename_comments,
        no_builtin_mappings: false,
        raw: false,
        quiet: false,
    })
}

pub fn java_levels(major: u16) -> (String, String) {
    match major {
        0..=45 => ("1.3".into(), "1.1".into()),
        46 => ("1.2".into(), "1.2".into()),
        47 => ("1.3".into(), "1.3".into()),
        48 => ("1.4".into(), "1.4".into()),
        49..=52 => (format!("1.{}", major - 44), format!("1.{}", major - 44)),
        _ => ((major - 44).to_string(), (major - 44).to_string()),
    }
}

fn same_file(source: &Path, destination: &Path) -> bool {
    if destination.canonicalize().is_ok_and(|path| path == source) {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(left), Ok(right)) = (fs::metadata(source), fs::metadata(destination)) {
            return left.dev() == right.dev() && left.ino() == right.ino();
        }
    }
    false
}

fn resource_entries(jar: &Path) -> Result<Vec<(usize, PathBuf)>> {
    let file = fs::File::open(jar).with_context(|| format!("open {}", jar.display()))?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mut entries = Vec::new();
    let mut names = BTreeSet::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name()?.into_owned();
        ensure!(
            !name.is_empty()
                && !name.contains(['\\', ':', '\0'])
                && name.split('/').all(|part| !matches!(part, "" | "." | "..")),
            "unsafe JAR resource path {name:?}"
        );
        ensure!(names.insert(name.clone()), "duplicate JAR entry {name:?}");
        ensure!(
            entry
                .unix_mode()
                .is_none_or(|mode| mode & 0o170000 != 0o120000),
            "symlink JAR entry {name:?}"
        );
        let mut header = [0_u8; 4];
        let mut read = 0;
        while read < 4 {
            let count = entry.read(&mut header[read..])?;
            if count == 0 {
                break;
            }
            read += count;
        }
        if name.ends_with(".class") && header == [0xca, 0xfe, 0xba, 0xbe] {
            continue;
        }
        ensure!(
            entry.size() <= 64 * 1024 * 1024,
            "resource {name:?} exceeds 64 MiB"
        );
        entries.push((index, name.into()));
    }
    Ok(entries)
}

pub fn extract(root: &Path) -> Result<()> {
    let project = Project::read(root)?;
    let jar = project.jar();
    let entries = resource_entries(&jar)?;
    let mut archive = zip::ZipArchive::new(fs::File::open(&jar)?)?;
    let destination = project.root.join("resources");
    ensure!(
        !jar.canonicalize()?
            .starts_with(crate::artifact_plan::resolved(&destination)?),
        "source JAR {} is inside resource output {}",
        jar.display(),
        destination.display()
    );
    output::reset_directory(&destination)?;
    for (index, path) in &entries {
        let mut entry = archive.by_index(*index)?;
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes)?;
        output::write_file(&destination.join(path), &bytes)?;
    }
    eprintln!(
        "Extracted {} resources into {}",
        entries.len(),
        destination.display()
    );
    Ok(())
}
