use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::Serialize;
use sporeflower::{
    api::{ApiCatalog, ApiOptions},
    input::{Input, source_path},
    java::{JavaOptions, decompile_class, domain_sources},
    mappings,
    naming::{NamePlan, RenameOptions, mapping_text},
    program::Program,
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use crate::{
    artifact_plan::{ArtifactPlan, resolved},
    cli::DecompileArgs,
    compile_inputs::{ApiInput, Classpath, CompileInputs},
    mapping_reports, output, project,
};

#[derive(Default, Serialize)]
struct Summary {
    input: PathBuf,
    output: PathBuf,
    classes: usize,
    failed_classes: usize,
    methods: usize,
    failed_methods: usize,
    fallback_methods: usize,
    elapsed_ms: u128,
    diagnostics: Vec<String>,
}

#[derive(Default)]
struct ClassResult {
    failed: bool,
    methods: usize,
    failed_methods: usize,
    fallback_methods: usize,
    diagnostics: Vec<String>,
}

pub fn run(args: DecompileArgs, artifacts: Option<&Path>) -> Result<bool> {
    let project = args
        .project
        .as_deref()
        .map(project::Project::read)
        .transpose()?;
    run_resolved(args, project.as_ref(), artifacts, true)
}

pub fn run_project(
    args: DecompileArgs,
    project: &project::Project,
    artifacts: &Path,
    raw: bool,
) -> Result<bool> {
    run_resolved(args, Some(project), Some(artifacts), !raw)
}

fn run_resolved(
    args: DecompileArgs,
    project: Option<&project::Project>,
    artifacts: Option<&Path>,
    project_mappings: bool,
) -> Result<bool> {
    anyhow::ensure!(args.threads > 0, "--threads must be positive");
    let start = Instant::now();
    let Loaded {
        input_path,
        mut input,
        mapping_inputs,
        libraries,
        selected_api,
        classpath,
        mapping_paths,
    } = Loaded::read(&args, project, project_mappings, artifacts.is_some())?;
    let output = resolved(
        &args
            .output
            .or(args.destination)
            .unwrap_or_else(|| PathBuf::from("decompiled")),
    )?;
    let mut summary = Summary {
        input: input_path,
        output: output.clone(),
        classes: input.classes().len(),
        diagnostics: std::mem::take(&mut input.warnings),
        ..Summary::default()
    };
    let mut program = Program::from_input(&input, &libraries)?;
    let mappings = mapping_inputs.resolve(&program, !args.no_semantic_mappings)?;
    let names = NamePlan::build_with_mapping_origins(
        &program,
        &RenameOptions {
            rename_obfuscated: args.rename_obfuscated,
        },
        &mappings.names,
        &mappings.origins,
    )?;
    summary
        .diagnostics
        .extend(mappings.diagnostics.iter().map(ToString::to_string));
    summary
        .diagnostics
        .extend(names.diagnostics.iter().cloned());
    let options = JavaOptions {
        mappings: Some(&mappings),
    };
    let source_paths = (0..program.own_count())
        .map(|id| source_path(Path::new(""), names.class_name(id)))
        .collect::<Result<Vec<_>>>()?;
    let domains = domain_sources(&mappings, &program, &names)?
        .into_iter()
        .map(|(name, source)| Ok((source_path(Path::new(""), &name)?, source)))
        .collect::<Result<Vec<_>>>()?;
    let mapping_path = PathBuf::from("mapping.tiny");
    let mut plan = ArtifactPlan::for_directory(&output)?;
    let stem = summary
        .input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("project");
    let mapped_jar = artifacts
        .map(|directory| plan.resolve_output(&directory.join(format!("{stem}-mapped.jar"))))
        .transpose()?;
    let artifact_mapping = artifacts
        .map(|directory| plan.resolve_output(&directory.join("mapping.tiny")))
        .transpose()?;
    let api_path = args
        .stubs
        .as_deref()
        .map(|path| plan.resolve_output(path))
        .transpose()?;
    let json_path = args
        .json
        .as_deref()
        .map(|path| plan.resolve_output(path))
        .transpose()?;
    let reports = args
        .mapping_report
        .as_ref()
        .map(|directory| {
            mapping_reports::NAMES
                .iter()
                .map(|name| plan.resolve_output(&directory.join(name)))
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    for path in source_paths
        .iter()
        .chain(domains.iter().map(|(path, _)| path))
    {
        plan.output("Java source", output.join(path))?;
    }
    if !args.no_mapping {
        plan.output("Tiny mapping", output.join(&mapping_path))?;
    }
    plan.output("compilation settings", output.join(CompileInputs::FILE))?;
    for (role, path) in [
        ("mapped JAR", &mapped_jar),
        ("artifact Tiny", &artifact_mapping),
        ("API", &api_path),
        ("JSON summary", &json_path),
    ] {
        if let Some(path) = path {
            plan.output(role, path)?;
        }
    }
    for path in &reports {
        plan.output("mapping report", path)?;
    }
    plan.protect_directory(&summary.input, "class", true)?;
    for path in classpath.paths() {
        plan.protect_directory(path, "class", true)?;
    }
    for path in &mapping_paths {
        plan.protect_directory(path, "map", false)?;
    }
    if let Some(project) = project {
        plan.protect_input(&project.root.join("j2me.toml"))?;
    }

    // Inputs, mappings, and every destination have been validated. Generated
    // sources are disposable; a failed write can be completed by rerunning.
    output::reset_directory(&output)?;
    if let Some(jar) = mapped_jar {
        let report =
            sporeflower::remap::remap_jar_with_mappings(&jar, &program, &names, &mappings)?;
        summary.diagnostics.extend(report.warnings);
    }
    if let Some(path) = artifact_mapping {
        let parent = path.parent().unwrap();
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        fs::write(&path, mapping_text(&program, &names)?)
            .with_context(|| format!("write {}", path.display()))?;
    }
    // Mapping validation must precede all artifact writes, including API stubs.
    let api = match (selected_api, api_path) {
        (Some(selected), Some(path)) => {
            selected.write_jar(&path)?;
            ApiInput::Exported(path)
        }
        (Some(_), None) => ApiInput::NotExported,
        (None, _) => ApiInput::Disabled,
    };
    let project_root = project
        .map(|project| project.root.as_path())
        .unwrap_or_else(|| {
            output
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
        });
    let compile_inputs = CompileInputs {
        api,
        classpath,
        bytecode_major: input
            .classes()
            .iter()
            .map(|c| c.major_version)
            .max()
            .unwrap(),
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(args.threads)
        .build()?;
    program.prepare_throws(&pool);
    let results: Vec<ClassResult> = pool.install(|| {
        (0..program.own_count())
            .into_par_iter()
            .map(|id| match decompile_class(&program, &names, id, &options) {
                Ok(class) => {
                    output::write_file(&output.join(&source_paths[id]), class.source.as_bytes())?;
                    Ok(ClassResult {
                        failed: false,
                        methods: class.total_methods,
                        failed_methods: class.failed_methods,
                        fallback_methods: class.fallback_methods,
                        diagnostics: class.diagnostics,
                    })
                }
                Err(error) => Ok(ClassResult {
                    failed: true,
                    diagnostics: vec![format!("{}: {error:#}", program.class(id).name)],
                    ..ClassResult::default()
                }),
            })
            .collect::<Result<Vec<_>>>()
    })?;
    for result in results {
        summary.failed_classes += usize::from(result.failed);
        summary.methods += result.methods;
        summary.failed_methods += result.failed_methods;
        summary.fallback_methods += result.fallback_methods;
        summary.diagnostics.extend(result.diagnostics);
    }
    if !args.no_mapping {
        output::write_file(
            &output.join(&mapping_path),
            mapping_text(&program, &names)?.as_bytes(),
        )?;
    }
    for (path, source) in domains {
        output::write_file(&output.join(path), source.as_bytes())?;
    }
    compile_inputs.write(&output, project_root)?;
    if let Some(directory) = args.mapping_report {
        mapping_reports::write(&directory, &program, &names, &mappings)?;
    }
    summary.elapsed_ms = start.elapsed().as_millis();
    if let Some(path) = json_path {
        let parent = path.parent().unwrap();
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        fs::write(&path, serde_json::to_vec_pretty(&summary)?)
            .with_context(|| format!("write {}", path.display()))?;
    }
    for diagnostic in &summary.diagnostics {
        eprintln!("{diagnostic}");
    }
    if !args.quiet {
        eprintln!(
            "{} classes, {} methods; {} failed classes, {} failed methods, {} fallback methods; {:.3}s",
            summary.classes,
            summary.methods,
            summary.failed_classes,
            summary.failed_methods,
            summary.fallback_methods,
            start.elapsed().as_secs_f64()
        );
    }
    Ok(summary.failed_classes == 0 && summary.failed_methods == 0)
}

/// Original-identity inputs, resolved once for analysis and source generation.
struct Loaded {
    input_path: PathBuf,
    input: Input,
    mapping_inputs: mappings::MappingInputs,
    libraries: Vec<sporeflower::classfile::ClassFile>,
    selected_api: Option<sporeflower::api::ApiSelection>,
    classpath: Classpath,
    mapping_paths: Vec<PathBuf>,
}

impl Loaded {
    fn read(
        args: &DecompileArgs,
        project: Option<&project::Project>,
        project_mappings: bool,
        remap: bool,
    ) -> Result<Self> {
        let mut mapping_paths = args.mappings.clone();
        let input_path = if let Some(project) = project {
            let directory = project.root.join("mappings");
            if project_mappings && directory.is_dir() {
                mapping_paths.insert(0, directory);
            }
            project.jar()
        } else {
            args.input
                .clone()
                .context("input or --project is required")?
        };
        let mut input = if remap {
            Input::read_for_remap(&input_path)?
        } else {
            Input::read(&input_path)?
        };
        let mapping_inputs = mappings::MappingInputs::read(&mapping_paths)?;
        let mut paths = project
            .map(|project| project.classpath())
            .unwrap_or_default();
        paths.extend(args.classpath.iter().cloned());
        let classpath = Classpath::resolve(paths)?;
        let mut explicit = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for path in classpath.paths() {
            let mut external = Input::read(path)?;
            input.warnings.append(&mut external.warnings);
            explicit.extend(
                external
                    .into_classes()
                    .into_iter()
                    .filter(|c| seen.insert(c.name.clone())),
            );
        }
        let mut libraries = Vec::new();
        let selected_api = if args.no_api {
            anyhow::ensure!(
                args.stubs.is_none(),
                "--stubs requires bundled APIs (remove --no-api)"
            );
            None
        } else {
            let mut selected = ApiCatalog::bundled()?.select_with_classpath(
                input.classes(),
                &explicit,
                &ApiOptions {
                    configuration: input.manifest.get("microedition-configuration").cloned(),
                    additional_roots: mapping_inputs
                        .required_api_roots(input.classes(), !args.no_semantic_mappings)?,
                    ..ApiOptions::default()
                },
            )?;
            input.warnings.append(&mut selected.warnings);
            libraries = std::mem::take(&mut selected.classes);
            Some(selected)
        };
        libraries.extend(explicit);
        Ok(Self {
            input_path,
            input,
            mapping_inputs,
            libraries,
            selected_api,
            classpath,
            mapping_paths,
        })
    }
}
