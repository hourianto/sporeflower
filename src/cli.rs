use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    version,
    about = "Native J2ME decompiler and mapping project toolkit",
    args_conflicts_with_subcommands = true,
    subcommand_precedence_over_arg = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
    #[command(flatten)]
    pub direct: DecompileArgs,
}

#[derive(Subcommand)]
pub enum Command {
    /// Decompile a JAR, class file, class directory, or existing mapping project.
    Decompile(DecompileArgs),
    /// Create a project, extract resources, and run remap.
    Init(crate::project::InitArgs),
    /// Regenerate mapped bytecode, Java sources, API stubs, and mapping reports.
    Remap(crate::project::RemapArgs),
    /// Recreate resources/ from the JAR's non-code entries.
    ExtractResources(crate::project::ProjectLocation),
    /// Compile decompiled Java against exported API stubs (development check).
    CompileStubs(crate::compiler::CompileArgs),
}

/// Decompile J2ME class files and JARs into Java source.
#[derive(clap::Args, Clone, Default)]
pub struct DecompileArgs {
    /// A JAR, class file, or directory containing class files.
    #[arg(conflicts_with = "project")]
    pub input: Option<PathBuf>,
    /// Read the original JAR and mappings from an existing j2me.toml project.
    #[arg(long, value_name = "DIRECTORY")]
    pub project: Option<PathBuf>,
    /// Source output directory; cleared before generation.
    pub destination: Option<PathBuf>,
    /// Source output directory; cleared before generation.
    #[arg(short, long, conflicts_with = "destination")]
    pub output: Option<PathBuf>,
    /// Additional library JARs or class directories; override bundled APIs, last entry wins.
    #[arg(short = 'c', long = "classpath")]
    pub classpath: Vec<PathBuf>,
    /// Maximum simultaneous workers. Defaults to min(CPU count, 8).
    #[arg(short = 'j', long, default_value_t = default_threads())]
    pub threads: usize,
    /// Do not load the bundled CLDC/MIDP and vendor API declarations.
    #[arg(long)]
    pub no_api: bool,
    /// Export the selected API declarations as a compilation-only JAR.
    #[arg(long, value_name = "JAR")]
    pub stubs: Option<PathBuf>,
    /// Write structured completion statistics and diagnostics.
    #[arg(long, value_name = "FILE")]
    pub json: Option<PathBuf>,
    /// Omit the original-to-source Tiny v2 name map.
    #[arg(long)]
    pub no_mapping: bool,
    /// Replace short obfuscated class names; illegal names are always repaired.
    #[arg(long)]
    pub rename_obfuscated: bool,
    /// Authored Java-like .map file or directory. May be repeated.
    #[arg(long, value_name = "PATH")]
    pub mappings: Vec<PathBuf>,
    /// Disable bundled API semantic contracts; authored maps still apply.
    #[arg(long = "no-builtin-mappings", alias = "no-semantic-mappings")]
    pub no_semantic_mappings: bool,
    /// Write mapping coverage, member-use priorities and semantic contracts.
    #[arg(long, value_name = "DIRECTORY")]
    pub mapping_report: Option<PathBuf>,
    /// Omit the completion summary; diagnostics are still printed.
    #[arg(short, long)]
    pub quiet: bool,
}

pub fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get().min(8))
}
