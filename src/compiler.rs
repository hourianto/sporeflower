//! Compilation is an optional development check; the native pipeline needs no JVM.
use crate::{
    artifact_plan::ArtifactPlan,
    compile_inputs::{ApiInput, Classpath, CompileInputs},
    output,
    project::{Project, ProjectLocation, java_levels},
};
use anyhow::{Context, Result, ensure};
use clap::{Args, ValueEnum};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Copy, Debug, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Javac,
    Legacy,
    Ecj,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompilerConfig {
    pub backend: Option<Backend>,
    #[serde(alias = "compiler_jar")]
    pub jar: Option<PathBuf>,
    #[serde(alias = "java_bin")]
    pub java: Option<PathBuf>,
    #[serde(alias = "javac_bin")]
    pub javac: Option<PathBuf>,
    pub java_release: Option<u16>,
    pub source_level: Option<String>,
    pub target_level: Option<String>,
}

#[derive(Args)]
pub struct CompileArgs {
    #[command(flatten)]
    location: ProjectLocation,
    /// Compiler backend; javac by default, legacy if --compiler-jar is supplied.
    #[arg(long, value_enum)]
    compiler: Option<Backend>,
    /// Path to the compiler JAR for the legacy or ecj backend.
    #[arg(long)]
    compiler_jar: Option<PathBuf>,
    /// javac executable for the javac backend (default: project setting or PATH).
    #[arg(long)]
    javac_bin: Option<PathBuf>,
    /// java executable for legacy/ecj backends (default: project setting or PATH).
    #[arg(long)]
    java_bin: Option<PathBuf>,
    /// Modern javac release (default: 8).
    #[arg(long)]
    java_release: Option<u16>,
    /// Source level for legacy/ecj; defaults to project settings.
    #[arg(long)]
    source_level: Option<String>,
    /// Bytecode target level for legacy/ecj; defaults to project settings.
    #[arg(long)]
    target_level: Option<String>,
    /// Source directory (default: project/decompiled).
    #[arg(long)]
    decompiled_src: Option<PathBuf>,
    /// Optional additional authored stub sources, compiled with project sources.
    #[arg(long)]
    stubs_src: Option<PathBuf>,
    /// Override the generated API JAR; required for sources without saved inputs.
    #[arg(long)]
    api_stubs: Option<PathBuf>,
    /// Build artifacts directory (default: project/out/compile-stubs).
    #[arg(long)]
    out_dir: Option<PathBuf>,
    /// Additional libraries; later entries override earlier and saved inputs.
    #[arg(short = 'c', long = "classpath")]
    classpath: Vec<PathBuf>,
    /// Maximum compiler errors to report.
    #[arg(long, default_value_t = 1000)]
    maxerrs: usize,
}

pub fn run(args: CompileArgs) -> Result<bool> {
    ensure!(args.maxerrs > 0, "--maxerrs must be positive");
    let project = Project::read(&args.location.project)?;
    let config = project.compiler();
    let backend = args.compiler.or(config.backend).unwrap_or_else(|| {
        if args.compiler_jar.is_some() || config.jar.is_some() {
            Backend::Legacy
        } else {
            Backend::Javac
        }
    });
    let sources_root = args
        .decompiled_src
        .unwrap_or_else(|| project.root.join("decompiled"));
    let mut sources = files(&sources_root, "java")?;
    if let Some(stubs) = args.stubs_src {
        sources.extend(files(&stubs, "java")?);
    }
    sources.sort();
    sources.dedup();
    ensure!(
        !sources.is_empty(),
        "no Java sources in {}; run sporeflower remap first",
        sources_root.display()
    );
    let inputs = match CompileInputs::read(&sources_root)? {
        Some(inputs) => inputs,
        None => CompileInputs {
            api: ApiInput::Exported(args.api_stubs.clone().with_context(|| {
                format!(
                    "no saved compilation inputs in {}; run sporeflower remap, generate direct sources with --stubs <api.jar>, or supply --api-stubs <api.jar> for custom sources",
                    sources_root.display()
                )
            })?),
            classpath: Classpath::resolve(project.classpath())?,
            bytecode_major: project.bytecode().major.unwrap_or(52),
        },
    };
    let api = match args.api_stubs {
        Some(path) => Some(path),
        None => match inputs.api {
            ApiInput::Exported(path) => Some(path),
            ApiInput::Disabled => None,
            ApiInput::NotExported => anyhow::bail!(
                "sources in {} were generated without exporting their API; regenerate with --stubs <api.jar> or supply --api-stubs <api.jar>",
                sources_root.display()
            ),
        },
    };
    let api = api
        .map(|path| {
            path.canonicalize().with_context(|| {
                format!(
                    "resolve API stubs {}; regenerate sources or supply --api-stubs <api.jar>",
                    path.display()
                )
            })
        })
        .transpose()?;
    let classpath = inputs.classpath.with_overrides(args.classpath)?;
    // Explicit declarations override bundled API declarations, including when
    // the user supplies a new SDK at compilation time.
    let mut compile_classpath = classpath.paths().to_vec();
    compile_classpath.extend(api);
    let output = args
        .out_dir
        .unwrap_or_else(|| project.root.join("out/compile-stubs"));
    let plan = ArtifactPlan::for_directory(&output.join("classes"))?;
    let mut command;
    let levels = match backend {
        Backend::Javac => {
            ensure!(
                args.compiler_jar.is_none(),
                "--compiler-jar requires --compiler legacy or ecj"
            );
            ensure!(
                args.source_level.is_none() && args.target_level.is_none(),
                "javac uses --java-release; source/target overrides require legacy or ecj"
            );
            let bin = args.javac_bin.unwrap_or_else(|| {
                configured_tool(&project.root, config.javac.as_deref(), "javac")
            });
            let version = Command::new(&bin)
                .arg("-version")
                .output()
                .with_context(|| {
                    format!(
                        "run {} -version; select a compiler with --javac-bin",
                        bin.display()
                    )
                })?;
            ensure!(
                version.status.success(),
                "{} -version failed: {}",
                bin.display(),
                String::from_utf8_lossy(&version.stderr)
            );
            let text = format!(
                "{} {}",
                String::from_utf8_lossy(&version.stdout),
                String::from_utf8_lossy(&version.stderr)
            );
            let major = javac_major(&text).context("could not determine javac version")?;
            ensure!(
                major >= 8,
                "compile-stubs requires Java 8 or newer javac; use the legacy backend for older targets"
            );
            let release = args.java_release.or(config.java_release).unwrap_or(8);
            ensure!(release > 0, "--java-release must be positive");
            command = Command::new(bin);
            if major <= 8 {
                ensure!(release == 8, "Java 8 javac requires --java-release 8");
                command.args(["-source", "8", "-target", "8"]);
            } else {
                command.args(["--release", &release.to_string()]);
            }
            command.arg("-Xlint:-options");
            format!("release {release}")
        }
        Backend::Legacy | Backend::Ecj => {
            ensure!(
                args.java_release.is_none(),
                "--java-release applies to javac; use --source-level/--target-level for legacy or ecj"
            );
            let jar = args
                .compiler_jar
                .or_else(|| config.jar.as_ref().map(|p| project.root.join(p)))
                .context(
                    "legacy/ecj compilation requires an explicit --compiler-jar or [compiler].jar",
                )?;
            ensure!(jar.is_file(), "compiler JAR not found: {}", jar.display());
            plan.protect_input(&jar)?;
            let major = project.bytecode().major.unwrap_or(inputs.bytecode_major);
            let inferred = java_levels(major);
            let source = args
                .source_level
                .or_else(|| config.source_level.clone())
                .or_else(|| project.bytecode().javac_source.clone())
                .unwrap_or(inferred.0);
            let target = args
                .target_level
                .or_else(|| config.target_level.clone())
                .or_else(|| project.bytecode().javac_target.clone())
                .unwrap_or(inferred.1);
            let java = args
                .java_bin
                .unwrap_or_else(|| configured_tool(&project.root, config.java.as_deref(), "java"));
            command = Command::new(java);
            command
                .arg("-cp")
                .arg(jar)
                .arg(match backend {
                    Backend::Legacy => "j2me.thirdparty.legacyjavac.Main",
                    _ => "org.eclipse.jdt.internal.compiler.batch.Main",
                })
                .args(["-source", &source, "-target", &target]);
            format!("source {source}, target {target}")
        }
    };
    let executable = Path::new(command.get_program());
    if executable.components().count() > 1 {
        plan.protect_input(executable)?;
    }
    fs::create_dir_all(&output)?;
    let output = output.canonicalize()?;
    if !matches!(backend, Backend::Javac) {
        // The exported API supplies CLDC java.lang, never the host JDK classes.
        command
            .arg("-bootclasspath")
            .arg(std::env::join_paths(&compile_classpath)?);
    }
    let list = output.join("sources.args");
    let source_list = sources
        .iter()
        .map(|path| argument_file_path(path))
        .collect::<Result<Vec<_>>>()?
        .join("\n");
    fs::write(&list, format!("{source_list}\n"))?;
    let classes = output.join("classes");
    for source in &sources {
        plan.protect_input(source)?;
    }
    for path in &compile_classpath {
        plan.protect_directory(path, "class", true)?;
    }
    command
        .args(["-encoding", "UTF-8", "-g:lines,source"])
        .arg(if matches!(backend, Backend::Ecj) {
            "-maxProblems"
        } else {
            "-Xmaxerrs"
        })
        .arg(args.maxerrs.to_string())
        .arg("-classpath")
        .arg(std::env::join_paths(&compile_classpath)?)
        .arg("-d")
        .arg(&classes)
        .arg(format!("@{}", list.display()));
    output::reset_directory(&classes)?;
    eprintln!(
        "Compile check: {backend:?}, {levels}, {} sources",
        sources.len()
    );
    let result = command
        .output()
        .with_context(|| format!("start {backend:?} compiler"))?;
    fs::write(output.join("compiler.stdout.log"), &result.stdout)?;
    fs::write(output.join("compiler.stderr.log"), &result.stderr)?;
    eprint!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "success": result.status.success(), "exit_code": result.status.code(), "sources": sources.len(), "compiler": format!("{backend:?}").to_lowercase(), "settings": levels,
            "source_directory": sources_root, "classpath": compile_classpath,
        }))?,
    )?;
    eprintln!(
        "Compile check {}: {}",
        if result.status.success() {
            "passed"
        } else {
            "failed"
        },
        output.display()
    );
    Ok(result.status.success())
}

fn configured_tool(root: &Path, value: Option<&Path>, default: &str) -> PathBuf {
    match value {
        Some(path) if path.components().count() > 1 => root.join(path),
        Some(path) => path.into(),
        None => PathBuf::from(default),
    }
}

fn files(root: &Path, extension: &str) -> Result<Vec<PathBuf>> {
    let root = root
        .canonicalize()
        .with_context(|| format!("resolve source directory {}", root.display()))?;
    let mut result = Vec::new();
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            result.extend(files(&entry.path(), extension)?);
        } else if kind.is_file() && entry.path().extension().is_some_and(|ext| ext == extension) {
            result.push(entry.path());
        }
    }
    result.sort();
    Ok(result)
}

fn argument_file_path(path: &Path) -> Result<String> {
    let text = path
        .to_str()
        .context("compiler argument files require UTF-8 source paths")?;
    ensure!(
        !text.contains(['\n', '\r']),
        "newline in compiler source path"
    );
    Ok(format!(
        "\"{}\"",
        text.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

fn javac_major(text: &str) -> Option<u16> {
    let mut words = text.split_whitespace();
    while let Some(word) = words.next() {
        if word == "javac" {
            let version = words.next()?;
            let version = version.strip_prefix("1.").unwrap_or(version);
            return version
                .split(|ch: char| !ch.is_ascii_digit())
                .next()?
                .parse()
                .ok();
        }
    }
    None
}
