//! Authored fixture workflow only: never execute a corpus application.
use crate::common::{checked, compile, javac};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_sporeflower"))
}
fn jar(path: &Path, classes: &Path, resources: &[(&str, &[u8])]) {
    let mut archive = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for class in crate::common::files(classes, "class") {
        archive
            .start_file(
                class.strip_prefix(classes).unwrap().to_str().unwrap(),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        archive.write_all(&fs::read(class).unwrap()).unwrap();
    }
    for (name, bytes) in resources {
        archive
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(bytes).unwrap();
    }
    archive.finish().unwrap();
}

struct Fixture {
    _temp: tempfile::TempDir,
    jar: PathBuf,
    classes: PathBuf,
    project: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("a.java");
        fs::write(
            &source,
            "package original; public class a { public static int a() { return 7; } }\n",
        )
        .unwrap();
        let classes = temp.path().join("original classes");
        compile(&[source], &classes, None);
        let input = temp.path().join("game.jar");
        jar(
            &input,
            &classes,
            &[
                (
                    "META-INF/MANIFEST.MF",
                    b"Manifest-Version: 1.0\r\nMIDlet-1: Fixture, /icon.class, original.a\r\n\r\n",
                ),
                ("icon.class", b"fake class resource"),
                ("assets/data.bin", &[0, 1, 2, 255]),
            ],
        );
        let project = temp.path().join("mapping project");
        fs::create_dir_all(project.join("mappings")).unwrap();
        fs::write(
            project.join("mappings/Names.map"),
            "package game; class Engine /* was original/a */ { int value() /* was a */; }\n",
        )
        .unwrap();
        Self {
            _temp: temp,
            jar: input,
            classes,
            project,
        }
    }
    fn init(&self) {
        checked(
            cli()
                .arg("init")
                .arg("--jar")
                .arg(&self.jar)
                .arg("--project")
                .arg(&self.project)
                .args(["--threads", "1"]),
        );
    }
    fn remap(&self) {
        checked(
            cli()
                .arg("remap")
                .arg("--project")
                .arg(&self.project)
                .args(["--threads", "1"]),
        );
    }
}

#[test]
fn failed_optional_report_leaves_generated_sources_and_artifacts() {
    let fixture = Fixture::new();
    fixture.init();
    let root = &fixture.project;
    let report = root.join("out/coverage.md");
    fs::rename(&report, root.join("previous-coverage.md")).unwrap();
    fs::create_dir(&report).unwrap();
    fs::write(
        root.join("mappings/Names.map"),
        "package next; class Changed /* was original/a */ {}\n",
    )
    .unwrap();
    let failed = cli()
        .arg("remap")
        .arg("--project")
        .arg(root)
        .args(["--threads", "1"])
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("coverage.md"));
    assert!(root.join("decompiled/next/Changed.java").is_file());
    assert!(!root.join("decompiled/game/Engine.java").exists());
    let inputs: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("decompiled/.sporeflower-compile.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(inputs["api"]["exported"], "../out/api-stubs.jar");
    assert_eq!(inputs["classpath"], serde_json::json!([]));
    assert!(inputs["bytecode_major"].is_number());
    let mapped = sporeflower::input::Input::read(&root.join("out/game-mapped.jar")).unwrap();
    assert_eq!(mapped.classes()[0].name, "next/Changed");
    assert!(
        fs::read_to_string(root.join("out/mapping.tiny"))
            .unwrap()
            .contains("next/Changed")
    );
    fs::rename(&report, root.join("report-obstruction")).unwrap();
    fixture.remap();
    assert!(report.is_file());
    assert!(root.join("decompiled/next/Changed.java").is_file());
}

#[test]
fn bare_cli_help_and_direct_input_remain_useful() {
    let result = checked(&mut cli());
    let help = String::from_utf8(result.stdout).unwrap();
    for command in [
        "init",
        "remap",
        "extract-resources",
        "compile-stubs",
        "decompile",
    ] {
        assert!(help.contains(command), "{help}");
    }
    let fixture = Fixture::new();
    let output = fixture._temp.path().join("direct");
    checked(
        cli()
            .arg(&fixture.jar)
            .arg("-o")
            .arg(&output)
            .args(["--threads", "1"]),
    );
    assert!(output.join("original/a.java").is_file());
    assert!(output.join("mapping.tiny").is_file());
}

#[test]
fn init_remap_compile_share_names_and_preserve_authored_inputs() {
    let fixture = Fixture::new();
    fixture.init();
    let root = &fixture.project;
    assert_eq!(
        fs::read_to_string(root.join("mappings/MAPPINGS.md")).unwrap(),
        include_str!("../docs/mappings.md")
    );
    assert_eq!(
        fs::read(root.join("game.jar")).unwrap(),
        fs::read(&fixture.jar).unwrap()
    );
    assert_eq!(
        fs::read(root.join("resources/icon.class")).unwrap(),
        b"fake class resource"
    );
    assert_eq!(
        fs::read(root.join("resources/assets/data.bin")).unwrap(),
        [0, 1, 2, 255]
    );
    assert!(!root.join("resources/original/a.class").exists());
    let source = fs::read_to_string(root.join("decompiled/game/Engine.java")).unwrap();
    assert!(source.contains("static int value()"), "{source}");
    for name in [
        "mapping.tiny",
        "coverage.md",
        "usage-priority.tsv",
        "semantic-summary.md",
        "api-stubs.jar",
        "decompile.json",
        "game-mapped.jar",
    ] {
        assert!(root.join("out").join(name).is_file(), "{name}");
    }
    let mapped = sporeflower::input::Input::read(&root.join("out/game-mapped.jar")).unwrap();
    assert_eq!(mapped.classes()[0].name, "game/Engine");
    assert!(
        mapped.classes()[0]
            .methods
            .iter()
            .any(|m| m.name == "value")
    );
    let compile = || {
        checked(
            cli()
                .arg("compile-stubs")
                .arg("--project")
                .arg(root)
                .arg("--javac-bin")
                .arg(javac()),
        );
    };
    compile();
    assert!(
        root.join("out/compile-stubs/classes/game/Engine.class")
            .is_file()
    );
    fs::write(
        root.join("out/compile-stubs/classes/game/Engine.class"),
        b"edited stale class",
    )
    .unwrap();
    fs::write(
        root.join("out/compile-stubs/classes/notes.txt"),
        b"untracked output",
    )
    .unwrap();
    fs::write(
        root.join("mappings/Names.map"),
        "package next; class Renamed /* was original/a */ { int answer() /* was a */; }\n",
    )
    .unwrap();
    fixture.remap();
    assert!(!root.join("decompiled/game/Engine.java").exists());
    assert!(root.join("decompiled/next/Renamed.java").exists());
    compile();
    assert!(
        !root
            .join("out/compile-stubs/classes/game/Engine.class")
            .exists()
    );
    assert!(
        root.join("out/compile-stubs/classes/next/Renamed.class")
            .exists()
    );
    assert!(!root.join("out/compile-stubs/classes/notes.txt").exists());
}

#[test]
fn compile_saved_sources_after_original_jar_moves() {
    let fixture = Fixture::new();
    fixture.init();
    let root = &fixture.project;
    let original = root.join("game.jar");
    fs::rename(&original, fixture._temp.path().join("moved-original.jar")).unwrap();
    checked(
        cli()
            .arg("compile-stubs")
            .arg("--project")
            .arg(root)
            .arg("--javac-bin")
            .arg(javac()),
    );
    assert!(
        root.join("out/compile-stubs/classes/game/Engine.class")
            .is_file()
    );
    for command in ["remap", "decompile", "extract-resources"] {
        let result = cli()
            .arg(command)
            .arg("--project")
            .arg(root)
            .output()
            .unwrap();
        assert!(!result.status.success(), "{command} accepted a missing JAR");
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(error.contains(original.to_str().unwrap()), "{error}");
    }
}

#[test]
fn init_requires_force_and_retains_unrelated_config_and_maps() {
    let fixture = Fixture::new();
    fixture.init();
    let config = fixture.project.join("j2me.toml");
    let original = fs::read_to_string(&config).unwrap();
    fs::write(&config, format!("author_note = 'retain me'\n{original}")).unwrap();
    let before = fs::read(&config).unwrap();
    let result = cli()
        .arg("init")
        .arg("--jar")
        .arg(&fixture.jar)
        .arg("--project")
        .arg(&fixture.project)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(fs::read(&config).unwrap(), before);
    checked(
        cli()
            .arg("init")
            .arg("--jar")
            .arg(&fixture.jar)
            .arg("--project")
            .arg(&fixture.project)
            .args(["--force", "--threads", "1"]),
    );
    assert!(fs::read_to_string(config).unwrap().contains("retain me"));
    assert!(fixture.project.join("decompiled/game/Engine.java").exists());
}

#[test]
fn init_recognizes_an_existing_hardlink_to_its_source_jar() {
    let fixture = Fixture::new();
    fs::hard_link(&fixture.jar, fixture.project.join("game.jar")).unwrap();
    let previous = fs::read(&fixture.jar).unwrap();
    fixture.init();
    assert_eq!(fs::read(&fixture.jar).unwrap(), previous);
    assert!(fixture.project.join("decompiled/game/Engine.java").exists());
}

#[test]
fn extraction_replaces_generated_resources_and_rejects_archive_traversal() {
    let fixture = Fixture::new();
    fixture.init();
    let resource = fixture.project.join("resources/assets/data.bin");
    fs::write(&resource, b"local edit").unwrap();
    checked(
        cli()
            .arg("extract-resources")
            .arg("--project")
            .arg(&fixture.project),
    );
    assert_eq!(fs::read(&resource).unwrap(), [0, 1, 2, 255]);
    let unrelated = fixture.project.join("resources/notes.txt");
    fs::write(&unrelated, b"untracked notes").unwrap();
    jar(
        &fixture.project.join("game.jar"),
        &fixture.classes,
        &[("new.dat", b"fresh")],
    );
    checked(
        cli()
            .arg("extract-resources")
            .arg("--project")
            .arg(&fixture.project),
    );
    assert!(!resource.exists());
    assert!(!unrelated.exists());
    assert!(!fixture.project.join("resources/icon.class").exists());
    let fresh = fixture.project.join("resources/new.dat");
    assert_eq!(fs::read(&fresh).unwrap(), b"fresh");
    jar(
        &fixture.project.join("game.jar"),
        &fixture.classes,
        &[("../escaped.bin", b"bad")],
    );
    let result = cli()
        .arg("extract-resources")
        .arg("--project")
        .arg(&fixture.project)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!fixture.project.join("escaped.bin").exists());
    assert_eq!(fs::read(&fresh).unwrap(), b"fresh");
}

#[test]
fn extraction_rejects_a_source_jar_inside_its_output_directory() {
    let fixture = Fixture::new();
    fixture.init();
    let input = fixture.project.join("resources/input.jar");
    fs::rename(fixture.project.join("game.jar"), &input).unwrap();
    fs::write(
        fixture.project.join("j2me.toml"),
        "jar = 'resources/input.jar'\n",
    )
    .unwrap();
    let before = fs::read(&input).unwrap();
    let result = cli()
        .arg("extract-resources")
        .arg("--project")
        .arg(&fixture.project)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("inside resource output"));
    assert_eq!(fs::read(input).unwrap(), before);
    assert_eq!(
        fs::read(fixture.project.join("resources/assets/data.bin")).unwrap(),
        [0, 1, 2, 255]
    );
}

#[test]
fn invalid_project_mapping_keeps_previous_artifacts() {
    let fixture = Fixture::new();
    fixture.init();
    let paths = [
        "decompiled/game/Engine.java",
        "decompiled/.sporeflower-compile.json",
        "out/game-mapped.jar",
        "out/api-stubs.jar",
        "out/mapping.tiny",
    ];
    let previous: Vec<_> = paths
        .iter()
        .map(|p| fs::read(fixture.project.join(p)).unwrap())
        .collect();
    fs::write(
        fixture.project.join("mappings/Names.map"),
        "class Missing /* was absent/Class */ {}\n",
    )
    .unwrap();
    let result = cli()
        .arg("remap")
        .arg("--project")
        .arg(&fixture.project)
        .output()
        .unwrap();
    assert!(!result.status.success());
    for (path, bytes) in paths.iter().zip(previous) {
        assert_eq!(fs::read(fixture.project.join(path)).unwrap(), bytes);
    }
}

#[test]
fn configured_compiler_is_honored_and_failure_clears_previous_classes() {
    let fixture = Fixture::new();
    fixture.init();
    let config = fixture.project.join("j2me.toml");
    let text = fs::read_to_string(&config).unwrap();
    fs::write(
        &config,
        format!(
            "{text}\n[compiler]\nbackend = 'javac'\njavac = {}\njava_release = 8\n",
            serde_json::to_string(javac().to_str().unwrap()).unwrap()
        ),
    )
    .unwrap();
    checked(
        cli()
            .arg("compile-stubs")
            .arg("--project")
            .arg(&fixture.project),
    );
    let class = fixture
        .project
        .join("out/compile-stubs/classes/game/Engine.class");
    assert!(class.is_file());
    fs::write(
        fixture.project.join("decompiled/game/Engine.java"),
        "not Java",
    )
    .unwrap();
    let result = cli()
        .arg("compile-stubs")
        .arg("--project")
        .arg(&fixture.project)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!class.exists());
    let report: serde_json::Value = serde_json::from_slice(
        &fs::read(fixture.project.join("out/compile-stubs/result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["success"], false);
}

#[test]
fn compilation_rejects_inputs_inside_its_class_output() {
    let fixture = Fixture::new();
    fixture.init();
    let classes = fixture.project.join("out/compile-stubs/classes");
    fs::create_dir_all(&classes).unwrap();
    let source = classes.join("Custom.java");
    fs::write(&source, "class Custom {}\n").unwrap();
    let library = classes.join("Library.class");
    fs::copy(fixture.classes.join("original/a.class"), &library).unwrap();
    for option in ["--decompiled-src", "--classpath"] {
        let result = cli()
            .arg("compile-stubs")
            .arg("--project")
            .arg(&fixture.project)
            .arg(option)
            .arg(&classes)
            .arg("--api-stubs")
            .arg(fixture.project.join("out/api-stubs.jar"))
            .arg("--javac-bin")
            .arg(javac())
            .output()
            .unwrap();
        assert!(!result.status.success(), "{option}");
        assert!(String::from_utf8_lossy(&result.stderr).contains("input"));
        assert_eq!(fs::read_to_string(&source).unwrap(), "class Custom {}\n");
        assert!(library.is_file());
    }
    let compiler = classes.join("compiler.jar");
    fs::write(&compiler, b"compiler input").unwrap();
    let result = cli()
        .arg("compile-stubs")
        .arg("--project")
        .arg(&fixture.project)
        .args(["--compiler", "ecj", "--compiler-jar"])
        .arg(&compiler)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("input"));
    assert_eq!(fs::read(compiler).unwrap(), b"compiler input");
}

#[test]
fn explicit_legacy_compiler_uses_exported_cldc_bootclasspath() {
    let Some(compiler) = crate::common::legacy_javac() else {
        return;
    };
    let fixture = Fixture::new();
    fixture.init();
    checked(
        cli()
            .arg("compile-stubs")
            .arg("--project")
            .arg(&fixture.project)
            .arg("--compiler-jar")
            .arg(compiler)
            .args(["--source-level", "1.3", "--target-level", "1.1"]),
    );
    let classes =
        sporeflower::input::Input::read(&fixture.project.join("out/compile-stubs/classes"))
            .unwrap();
    assert_eq!(classes.classes()[0].major_version, 45);
}

#[test]
fn configured_classpath_and_api_follow_a_moved_project() {
    let fixture = Fixture::new();
    let sdk_source = fixture._temp.path().join("Utility.java");
    fs::write(
        &sdk_source,
        "package sdk; public class Utility { public static int number() { return 3; } }\n",
    )
    .unwrap();
    let sdk_classes = fixture._temp.path().join("sdk classes");
    compile(&[sdk_source], &sdk_classes, None);
    let source = fixture._temp.path().join("a.java");
    fs::write(&source, "package original; public class a { public static int a() { return sdk.Utility.number(); } }\n").unwrap();
    compile(&[source], &fixture.classes, Some(&sdk_classes));
    jar(&fixture.jar, &fixture.classes, &[]);
    jar(&fixture.project.join("sdk.jar"), &sdk_classes, &[]);
    // Init with --force must keep project classpath settings already authored.
    fs::write(
        fixture.project.join("j2me.toml"),
        "jar = 'game.jar'\nclasspath = ['sdk.jar']\n",
    )
    .unwrap();
    checked(
        cli()
            .arg("init")
            .arg("--jar")
            .arg(&fixture.jar)
            .arg("--project")
            .arg(&fixture.project)
            .args(["--force", "--threads", "1"]),
    );
    let moved = fixture._temp.path().join("moved project");
    fs::rename(&fixture.project, &moved).unwrap();
    checked(
        cli()
            .arg("compile-stubs")
            .arg("--project")
            .arg(&moved)
            .arg("--javac-bin")
            .arg(javac()),
    );
    assert!(
        moved
            .join("out/compile-stubs/classes/game/Engine.class")
            .exists()
    );
}

#[test]
fn compile_uses_the_api_exported_for_external_semantic_mappings() {
    let fixture = Fixture::new();
    let source = fixture._temp.path().join("a.java");
    fs::write(
        &source,
        "package original; public class a { public static int a() { return 1; } }\n",
    )
    .unwrap();
    compile(&[source], &fixture.classes, None);
    jar(&fixture.jar, &fixture.classes, &[]);
    fixture.init();
    fs::rename(
        fixture.project.join("mappings/Names.map"),
        fixture.project.join("mappings/Names.saved"),
    )
    .unwrap();
    let extra = fixture._temp.path().join("external.map");
    fs::write(
        &extra,
        "package game; import javax.microedition.lcdui.CanvasGameAction; class Engine /* was original/a */ { @Domain(CanvasGameAction.class) int value() /* was a */; }\n",
    )
    .unwrap();
    checked(
        cli()
            .arg("remap")
            .arg("--project")
            .arg(&fixture.project)
            .arg("--mappings")
            .arg(&extra)
            .args(["--threads", "1"]),
    );
    let source = fs::read_to_string(fixture.project.join("decompiled/game/Engine.java")).unwrap();
    assert!(source.contains("Canvas.UP"), "{source}");
    // Compilation checks the source already generated, even if the author has
    // since begun a mapping edit that is not valid enough to regenerate yet.
    fs::write(
        fixture.project.join("mappings/Names.map"),
        "class Missing /* was absent/Class */ {}\n",
    )
    .unwrap();
    checked(
        cli()
            .arg("compile-stubs")
            .arg("--project")
            .arg(&fixture.project)
            .arg("--javac-bin")
            .arg(javac()),
    );
    let classes =
        sporeflower::input::Input::read(&fixture.project.join("out/compile-stubs/classes"))
            .unwrap();
    assert!(
        classes
            .classes()
            .iter()
            .any(|class| class.name == "game/Engine")
    );
}

#[test]
fn classpath_overrides_have_the_same_precedence_in_generation_and_compilation() {
    let fixture = Fixture::new();
    let sdk_source = fixture._temp.path().join("Utility.java");
    fs::write(
        &sdk_source,
        "package sdk; public class Utility { public static int old() { return 0; } }\n",
    )
    .unwrap();
    let old_classes = fixture._temp.path().join("old sdk");
    compile(std::slice::from_ref(&sdk_source), &old_classes, None);
    let old_sdk = fixture.project.join("sdk-old.jar");
    jar(&old_sdk, &old_classes, &[]);
    fs::write(
        &sdk_source,
        "package sdk; public class Utility { public static int number() { return 3; } }\n",
    )
    .unwrap();
    let new_classes = fixture._temp.path().join("new sdk");
    compile(&[sdk_source], &new_classes, None);
    let new_sdk = fixture._temp.path().join("sdk-new.jar");
    jar(&new_sdk, &new_classes, &[]);
    let source = fixture._temp.path().join("a.java");
    fs::write(&source, "package original; public class a { public static int a() { return sdk.Utility.number(); } }\n").unwrap();
    compile(&[source], &fixture.classes, Some(&new_classes));
    jar(&fixture.jar, &fixture.classes, &[]);
    fixture.init();
    let config = fixture.project.join("j2me.toml");
    let text = fs::read_to_string(&config).unwrap();
    fs::write(&config, format!("classpath = ['sdk-old.jar']\n{text}")).unwrap();
    checked(
        cli()
            .arg("remap")
            .arg("--project")
            .arg(&fixture.project)
            .arg("--classpath")
            .arg(&new_sdk)
            .args(["--threads", "1"]),
    );
    // Generation-time CLI inputs survive into a separate compile invocation.
    checked(
        cli()
            .arg("compile-stubs")
            .arg("--project")
            .arg(&fixture.project)
            .arg("--javac-bin")
            .arg(javac()),
    );
    // Explicit compiler overrides still win; repeated paths keep their latest
    // position instead of accidentally reviving the earlier SDK declaration.
    checked(
        cli()
            .arg("compile-stubs")
            .arg("--project")
            .arg(&fixture.project)
            .arg("--classpath")
            .arg(&old_sdk)
            .arg("--classpath")
            .arg(&new_sdk)
            .arg("--javac-bin")
            .arg(javac()),
    );
    let failed = cli()
        .arg("compile-stubs")
        .arg("--project")
        .arg(&fixture.project)
        .arg("--classpath")
        .arg(&old_sdk)
        .arg("--javac-bin")
        .arg(javac())
        .output()
        .unwrap();
    assert!(!failed.status.success());
    let errors = String::from_utf8_lossy(&failed.stderr);
    assert!(errors.contains("number()"), "{errors}");
}

#[test]
fn custom_source_directories_require_explicit_inputs_and_direct_exports_supply_them() {
    let fixture = Fixture::new();
    fixture.init();
    let custom = fixture._temp.path().join("custom sources");
    fs::create_dir(&custom).unwrap();
    fs::write(custom.join("Custom.java"), "class Custom {}\n").unwrap();
    let failed = cli()
        .arg("compile-stubs")
        .arg("--project")
        .arg(&fixture.project)
        .arg("--decompiled-src")
        .arg(&custom)
        .output()
        .unwrap();
    assert!(!failed.status.success());
    let errors = String::from_utf8_lossy(&failed.stderr);
    assert!(errors.contains("no saved compilation inputs"), "{errors}");
    assert!(errors.contains("--api-stubs"), "{errors}");
    checked(
        cli()
            .arg("compile-stubs")
            .arg("--project")
            .arg(&fixture.project)
            .arg("--decompiled-src")
            .arg(&custom)
            .arg("--api-stubs")
            .arg(fixture.project.join("out/api-stubs.jar"))
            .arg("--javac-bin")
            .arg(javac()),
    );
    // A direct command's source directory carries its own API export, separate
    // from the project's remapped sources and API artifact.
    let direct = fixture._temp.path().join("direct sources");
    let direct_api = fixture._temp.path().join("direct-api.jar");
    checked(
        cli()
            .arg(&fixture.jar)
            .arg("--output")
            .arg(&direct)
            .arg("--stubs")
            .arg(&direct_api)
            .args(["--threads", "1"]),
    );
    fs::rename(
        fixture.project.join("out/api-stubs.jar"),
        fixture.project.join("out/unused-api.jar"),
    )
    .unwrap();
    checked(
        cli()
            .arg("compile-stubs")
            .arg("--project")
            .arg(&fixture.project)
            .arg("--decompiled-src")
            .arg(&direct)
            .arg("--javac-bin")
            .arg(javac()),
    );
    assert!(
        fixture
            .project
            .join("out/compile-stubs/classes/original/a.class")
            .is_file()
    );
}

#[cfg(unix)]
#[test]
fn extraction_replaces_resource_symlinks_without_touching_their_targets() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    fixture.init();
    let destination = fixture.project.join("resources/assets/data.bin");
    let outside = fixture._temp.path().join("outside.bin");
    fs::rename(&destination, &outside).unwrap();
    symlink(&outside, &destination).unwrap();
    let before = fs::read(&outside).unwrap();
    checked(
        cli()
            .arg("extract-resources")
            .arg("--project")
            .arg(&fixture.project),
    );
    assert_eq!(fs::read(outside).unwrap(), before);
    assert!(!fs::symlink_metadata(&destination).unwrap().is_symlink());
    assert_eq!(fs::read(destination).unwrap(), [0, 1, 2, 255]);
}
