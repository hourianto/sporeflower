//! Regenerate an authored mapping project without running its bytecode.
use crate::common::checked;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

struct Case {
    _work: tempfile::TempDir,
    input: PathBuf,
    output: PathBuf,
    mapping: PathBuf,
}

impl Case {
    fn new() -> Self {
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join("input");
        let output = work.path().join("generated");
        let mapping = work.path().join("Names.map");
        fs::create_dir(&input).unwrap();
        fs::write(
            input.join("Reads.class"),
            include_bytes!("fixtures/legacy/classes/pkg/TestConsecutiveReads.class"),
        )
        .unwrap();
        fs::write(
            input.join("Accumulator.class"),
            include_bytes!("fixtures/legacy/classes/pkg/TestStackSavedAccumulator.class"),
        )
        .unwrap();
        Self {
            _work: work,
            input,
            output,
            mapping,
        }
    }

    fn command(&self) -> Command {
        self.command_for(&self.output)
    }

    fn command_for(&self, output: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sporeflower"));
        command
            .arg(&self.input)
            .arg("-o")
            .arg(output)
            .arg("--mappings")
            .arg(&self.mapping)
            .args(["--no-api", "--threads", "2"]);
        command
    }

    fn render(&self, mapping: &str) {
        fs::write(&self.mapping, mapping).unwrap();
        checked(&mut self.command());
    }
}

const FIRST: &str = r#"
package before;
class First /* was pkg/TestConsecutiveReads */ {}
class Edited /* was pkg/TestStackSavedAccumulator */ {}
@ValueDomain interface OldDomain { int VALUE = 1; }
@ValueDomain interface EditedDomain { int VALUE = 2; }
"#;

#[test]
fn regeneration_clears_stale_edited_and_unrelated_files() {
    let case = Case::new();
    case.render(FIRST);
    for name in ["Edited", "EditedDomain"] {
        let path = case.output.join(format!("before/{name}.java"));
        fs::write(&path, "// local edit\n").unwrap();
    }
    fs::write(
        case.output.join("before/HandWritten.java"),
        "// user source\n",
    )
    .unwrap();
    fs::write(case.output.join("HandWritten.class"), b"unrelated binary").unwrap();
    fs::create_dir_all(case.output.join("unrelated/nested")).unwrap();
    fs::write(case.output.join("unrelated/nested/notes.txt"), b"old notes").unwrap();
    case.render(
        r#"
package after;
class Second /* was pkg/TestConsecutiveReads */ {}
class RenamedEdited /* was pkg/TestStackSavedAccumulator */ {}
@ValueDomain interface NewDomain { int VALUE = 1; }
"#,
    );
    for stale in ["before", "unrelated", "HandWritten.class"] {
        assert!(!case.output.join(stale).exists(), "retained {stale}");
    }
    for name in ["Second", "RenamedEdited", "NewDomain"] {
        assert!(case.output.join(format!("after/{name}.java")).is_file());
    }
    let settings: serde_json::Value =
        serde_json::from_slice(&fs::read(case.output.join(".sporeflower-compile.json")).unwrap())
            .unwrap();
    assert_eq!(settings["api"], "disabled");
    assert!(settings["classpath"].is_array());
    assert!(settings["bytecode_major"].is_number());
}

#[test]
fn invalid_mapping_leaves_the_previous_generation_intact() {
    let case = Case::new();
    case.render(FIRST);
    let stubs = case.output.join("api.jar");
    fs::write(&stubs, b"previous API export").unwrap();
    let paths = [
        "before/First.java",
        "mapping.tiny",
        ".sporeflower-compile.json",
        "api.jar",
    ];
    let before: Vec<_> = paths
        .iter()
        .map(|path| fs::read(case.output.join(path)).unwrap())
        .collect();
    fs::write(
        &case.mapping,
        "package broken; class Missing /* was no/such/Class */ {}",
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_sporeflower"))
        .arg(&case.input)
        .arg("-o")
        .arg(&case.output)
        .arg("--mappings")
        .arg(&case.mapping)
        .arg("--stubs")
        .arg(&stubs)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("no/such/Class"));
    for (path, contents) in paths.iter().zip(before) {
        assert_eq!(fs::read(case.output.join(path)).unwrap(), contents);
    }
    assert!(!case.output.join("broken").exists());
}

#[test]
fn disabling_tiny_output_removes_even_an_edited_mapping() {
    let case = Case::new();
    case.render(FIRST);
    let map = case.output.join("mapping.tiny");
    fs::write(&map, "user-edited name map\n").unwrap();
    checked(case.command().arg("--no-mapping"));
    assert!(!map.exists());
}

#[test]
fn old_obstructions_are_removed_before_generation() {
    let case = Case::new();
    fs::create_dir_all(case.output.join("mapping.tiny/nested")).unwrap();
    fs::write(case.output.join("before"), b"old contents").unwrap();
    case.render(FIRST);
    assert!(case.output.join("mapping.tiny").is_file());
    assert!(case.output.join("before/First.java").is_file());
}

#[cfg(unix)]
#[test]
fn old_source_directory_symlinks_are_removed_before_writing() {
    use std::os::unix::fs::symlink;
    let case = Case::new();
    let outside = case._work.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("First.java"), b"untouched").unwrap();
    fs::create_dir(&case.output).unwrap();
    symlink(&outside, case.output.join("before")).unwrap();
    case.render(FIRST);
    assert_eq!(fs::read(outside.join("First.java")).unwrap(), b"untouched");
    assert!(
        fs::read_to_string(case.output.join("before/First.java"))
            .unwrap()
            .contains("class First")
    );
    assert!(
        !fs::symlink_metadata(case.output.join("before"))
            .unwrap()
            .is_symlink()
    );
}

#[test]
fn summary_cannot_replace_compilation_settings_or_generated_source() {
    let case = Case::new();
    fs::write(&case.mapping, FIRST).unwrap();
    for target in [
        ".sporeflower-compile.json",
        "before/First.java",
        "mapping.tiny",
    ] {
        let failed = case
            .command()
            .arg("--json")
            .arg(case.output.join(target))
            .output()
            .unwrap();
        assert!(!failed.status.success(), "accepted {target}");
        assert!(String::from_utf8_lossy(&failed.stderr).contains("output collision"));
        assert!(!case.output.exists());
    }
    case.render(FIRST);
    let settings = case.output.join(".sporeflower-compile.json");
    let before = fs::read(&settings).unwrap();
    let failed = case
        .command()
        .arg("--json")
        .arg(&settings)
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert_eq!(fs::read(settings).unwrap(), before);
    checked(&mut case.command());
}

#[test]
fn api_reports_summary_and_inputs_share_one_collision_plan() {
    let case = Case::new();
    fs::write(&case.mapping, FIRST).unwrap();
    let api = case._work.path().join("api.jar");
    let reports = case._work.path().join("reports");
    let source = case.input.join("Reads.class");
    let mapping_before = fs::read(&case.mapping).unwrap();
    let source_before = fs::read(&source).unwrap();
    for (stubs, summary, report) in [
        (api.clone(), api.clone(), None),
        (
            reports.join("coverage.md"),
            api.clone(),
            Some(reports.clone()),
        ),
        (
            api.clone(),
            reports.join("semantic-summary.md"),
            Some(reports.clone()),
        ),
        (source.clone(), api.clone(), None),
        (api.clone(), case.mapping.clone(), None),
        (case.output.join("before"), api.clone(), None),
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sporeflower"));
        command
            .arg(&case.input)
            .arg("-o")
            .arg(&case.output)
            .arg("--mappings")
            .arg(&case.mapping)
            .arg("--stubs")
            .arg(stubs)
            .arg("--json")
            .arg(summary)
            .args(["--threads", "1"]);
        if let Some(report) = report {
            command.arg("--mapping-report").arg(report);
        }
        let failed = command.output().unwrap();
        assert!(!failed.status.success());
        assert!(
            String::from_utf8_lossy(&failed.stderr).contains("output collision"),
            "{}",
            String::from_utf8_lossy(&failed.stderr)
        );
        assert!(!case.output.exists());
        assert!(!api.exists());
        assert!(!reports.exists());
        assert_eq!(fs::read(&case.mapping).unwrap(), mapping_before);
        assert_eq!(fs::read(&source).unwrap(), source_before);
    }
}

#[cfg(unix)]
#[test]
fn summary_aliases_of_read_inputs_are_rejected_without_writes() {
    use std::os::unix::fs::symlink;
    let case = Case::new();
    fs::write(&case.mapping, FIRST).unwrap();
    let original = fs::read(&case.mapping).unwrap();
    let hard = case._work.path().join("hard.json");
    let symbolic = case._work.path().join("symbolic.json");
    fs::hard_link(&case.mapping, &hard).unwrap();
    symlink(&case.mapping, &symbolic).unwrap();
    for alias in [hard, symbolic] {
        let failed = case.command().arg("--json").arg(alias).output().unwrap();
        assert!(!failed.status.success());
        assert!(String::from_utf8_lossy(&failed.stderr).contains("output collision"));
        assert!(!case.output.exists());
        assert_eq!(fs::read(&case.mapping).unwrap(), original);
    }
}

#[test]
fn case_only_source_rename_uses_the_new_spelling() {
    let case = Case::new();
    let mapping = "package game; class foo /* was pkg/TestConsecutiveReads */ {}";
    case.render(mapping);
    case.render(&mapping.replace("class foo", "class Foo"));
    let source = case.output.join("game/Foo.java");
    assert!(fs::read_to_string(&source).unwrap().contains("class Foo"));
    let filenames: Vec<_> = fs::read_dir(case.output.join("game"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(filenames.iter().any(|name| name == "Foo.java"));
    assert!(!filenames.iter().any(|name| name == "foo.java"));
}

#[test]
fn summary_can_reuse_a_stale_source_path_after_clearing() {
    let case = Case::new();
    case.render(FIRST);
    let stale = case.output.join("before/First.java");
    fs::write(&case.mapping, FIRST.replace("before", "after")).unwrap();
    checked(case.command().arg("--json").arg(&stale));
    let summary: serde_json::Value = serde_json::from_slice(&fs::read(stale).unwrap()).unwrap();
    assert_eq!(summary["classes"], 2);
    assert!(case.output.join("after/First.java").is_file());
}

#[test]
fn clearing_cannot_remove_source_or_mapping_inputs() {
    let mut case = Case::new();
    case.render(FIRST);
    let source = case.input.join("Reads.class");
    let original = fs::read(&source).unwrap();
    for root in [&case.input, case._work.path()] {
        let failed = case.command_for(root).output().unwrap();
        assert!(!failed.status.success());
        assert!(String::from_utf8_lossy(&failed.stderr).contains("output collision"));
        assert_eq!(fs::read(&source).unwrap(), original);
        assert!(case.output.join("before/First.java").is_file());
    }
    case.mapping = case.output.join("Names.map");
    fs::write(&case.mapping, FIRST).unwrap();
    let failed = case.command().output().unwrap();
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("output collision"));
    assert_eq!(fs::read_to_string(&case.mapping).unwrap(), FIRST);
    assert!(case.output.join("before/First.java").is_file());
}

#[test]
fn clearing_cannot_remove_a_classpath_directory() {
    let case = Case::new();
    case.render(FIRST);
    let classpath = case.output.join("classpath");
    fs::create_dir(&classpath).unwrap();
    let class = classpath.join("Reads.class");
    fs::copy(case.input.join("Reads.class"), &class).unwrap();
    let original = fs::read(&class).unwrap();
    let failed = case
        .command()
        .arg("--classpath")
        .arg(&classpath)
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("output collision"));
    assert_eq!(fs::read(class).unwrap(), original);
    assert!(case.output.join("before/First.java").is_file());
}

#[test]
fn api_summary_and_reports_can_live_in_the_generated_directory() {
    let case = Case::new();
    fs::write(&case.mapping, FIRST).unwrap();
    let api = case.output.join("api.jar");
    let summary = case.output.join("summary.json");
    let reports = case.output.join("reports");
    for _ in 0..2 {
        checked(
            Command::new(env!("CARGO_BIN_EXE_sporeflower"))
                .arg(&case.input)
                .arg("-o")
                .arg(&case.output)
                .arg("--mappings")
                .arg(&case.mapping)
                .arg("--stubs")
                .arg(&api)
                .arg("--json")
                .arg(&summary)
                .arg("--mapping-report")
                .arg(&reports),
        );
        assert!(api.is_file());
        assert!(summary.is_file());
        assert!(reports.join("coverage.md").is_file());
        assert!(case.output.join("before/First.java").is_file());
    }
}
