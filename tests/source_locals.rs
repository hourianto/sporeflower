//! Source-level scopes retain scheduled effects and cross-scope phi bindings.
use crate::common::checked;
use crate::common::source::method;
use std::{fs, path::Path, process::Command};

fn compile(source: &Path, output: &Path) {
    fs::create_dir_all(output).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(output)
            .arg(source),
    );
}

fn standalone_int(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("int v") && line.ends_with(';') && !line.contains('=')
}

#[test]
fn declarations_follow_first_assignment_without_moving_effects_or_escaping_scopes() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let generated = work.path().join("generated");
    let rebuilt = work.path().join("rebuilt");
    let api = work.path().join("api.jar");
    compile(
        Path::new("tests/fixtures/java/SourceLocals.java"),
        &original,
    );
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&generated)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let source_path = generated.join("SourceLocals.java");
    let source = fs::read_to_string(&source_path).unwrap();

    let straight = method(&source, "straight");
    assert!(!straight.lines().any(standalone_int), "{straight}");
    let declaration = straight
        .find("int v")
        .expect("shared call result needs a local");
    assert!(
        straight.find("step(7)").unwrap() < declaration,
        "{straight}"
    );
    assert!(
        straight[declaration..].starts_with("int v") && straight[declaration..].contains(" = ")
    );

    let loop_body = method(&source, "loop");
    assert!(!loop_body.lines().any(standalone_int), "{loop_body}");
    assert!(
        loop_body
            .lines()
            .any(|line| line.starts_with("            int v")),
        "{loop_body}"
    );

    let guarded = method(&source, "guardedLong");
    assert_eq!(guarded.matches(" = 0L;").count(), 1, "{guarded}");
    let default = guarded.find(" = 0L;").unwrap();
    assert!(default < guarded.find("step(12)").unwrap(), "{guarded}");
    assert!(
        guarded.find("try {").unwrap() < guarded.find("step(p0)").unwrap(),
        "{guarded}"
    );

    let reference = method(&source, "guardedReference");
    assert_eq!(reference.matches(" = null;").count(), 1, "{reference}");
    assert!(
        reference.find("try {").unwrap() < reference.find("String.valueOf").unwrap(),
        "{reference}"
    );

    let monitor = method(&source, "guardedMonitor");
    assert!(
        monitor.find(" = 17;").unwrap() < monitor.find("synchronized (").unwrap(),
        "{monitor}"
    );
    assert!(
        monitor.find("synchronized (").unwrap() < monitor.find("step(p0)").unwrap(),
        "{monitor}"
    );

    compile(&source_path, &rebuilt);
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("SourceLocals"),
    )
    .stdout;
    let actual = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&rebuilt)
            .arg("SourceLocals"),
    )
    .stdout;
    assert_eq!(expected, actual);

    if let Some(legacy) = crate::common::legacy_javac() {
        let output = work.path().join("legacy");
        fs::create_dir_all(&output).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&output)
                .arg(&source_path),
        );
        assert_eq!(
            expected,
            checked(
                Command::new(crate::common::java())
                    .arg("-cp")
                    .arg(output)
                    .arg("SourceLocals")
            )
            .stdout
        );
    }
}
