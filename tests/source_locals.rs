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

    let delayed = method(&source, "delayedBranches");
    let declaration = delayed
        .lines()
        .find(|line| standalone_int(line))
        .expect("both statement branches write one result");
    assert!(
        delayed.find("step(19)").unwrap() < delayed.find(declaration).unwrap(),
        "{delayed}"
    );
    assert!(
        delayed.find(declaration).unwrap() < delayed.find("if (").unwrap(),
        "{delayed}"
    );

    let optional = method(&source, "optionalLiteral");
    assert!(!optional.contains("else"), "{optional}");
    assert_eq!(optional.matches(" = 0;").count(), 1, "{optional}");
    assert!(
        optional.find(" = 0;").unwrap() < optional.find("step(24)").unwrap(),
        "{optional}"
    );

    let single = method(&source, "singleUseLiteral");
    assert!(!single.contains("else"), "{single}");
    assert_eq!(single.matches(" = 0;").count(), 1, "{single}");
    assert!(
        single.find(" = 0;").unwrap() < single.find("step(28)").unwrap(),
        "{single}"
    );
    assert!(single.contains("step(29)"), "{single}");

    // A call-valued arm can still be a conditional expression. Its effect is
    // lazy and belongs to the result, unlike the independent step above.
    let call = method(&source, "optionalCall");
    assert!(call.contains('?'), "{call}");
    assert_eq!(call.matches(';').count(), 1, "{call}");
    assert!(!call.lines().any(standalone_int), "{call}");

    for name in ["singleUseLoop", "singleUseNested"] {
        let body = method(&source, name);
        assert!(!body.contains("else"), "{body}");
        assert!(!body.lines().any(standalone_int), "{body}");
        assert_eq!(body.matches(" = -1;").count(), 1, "{body}");
        assert!(
            body.find(" = -1;").unwrap() < body.find("if (").unwrap(),
            "{body}"
        );
    }

    // A real nested ternary is already recovered into one expression block.
    let nested_call = method(&source, "optionalNestedCall");
    assert_eq!(nested_call.matches('?').count(), 2, "{nested_call}");
    assert_eq!(nested_call.matches(';').count(), 1, "{nested_call}");

    let observed = method(&source, "singleUseObserved");
    assert!(!observed.contains("else"), "{observed}");
    assert_eq!(observed.matches(" = null;").count(), 1, "{observed}");
    assert!(
        observed.find(" = null;").unwrap() < observed.find("String.valueOf").unwrap(),
        "{observed}"
    );

    let pair = method(&source, "singleUsePair");
    assert_eq!(pair.matches(" = 0;").count(), 1, "{pair}");
    assert!(
        pair.find(" = 0;").unwrap() < pair.find("if (").unwrap(),
        "{pair}"
    );

    let repeated = method(&source, "repeatedLiteral");
    assert!(!repeated.contains("else"), "{repeated}");
    assert_eq!(repeated.matches(" = 3;").count(), 1, "{repeated}");
    assert!(
        repeated.find("for (").unwrap() < repeated.find(" = 3;").unwrap(),
        "{repeated}"
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

#[test]
fn captured_literal_does_not_publish_a_delayed_slot_store_before_a_throw() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let generated = work.path().join("generated");
    let rebuilt = work.path().join("rebuilt");
    let api = work.path().join("api.jar");
    fs::create_dir_all(&original).unwrap();
    fs::copy(
        "tests/fixtures/delayed_literal_store/classes/DelayedLiteralStore.class",
        original.join("DelayedLiteralStore.class"),
    )
    .unwrap();
    crate::common::compile(
        &["tests/fixtures/delayed_literal_store/DelayedLiteralStoreDriver.java".into()],
        &original,
        Some(&original),
    );
    let expected = crate::common::execute(&original, "DelayedLiteralStoreDriver");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&generated)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let source = fs::read_to_string(generated.join("DelayedLiteralStore.java")).unwrap();
    let run = method(&source, "run");
    assert!(
        run.find(" = 7;")
            .expect("the multi-use phi retains its literal seed")
            < run.find("DelayedLiteralStoreDriver.step(").unwrap(),
        "{run}"
    );
    let sources = crate::common::sources(&generated);
    crate::common::compile(&sources, &rebuilt, None);
    assert_eq!(
        expected,
        crate::common::execute(&rebuilt, "DelayedLiteralStoreDriver")
    );
    let legacy = work.path().join("legacy");
    if crate::common::legacy_compile(&sources, &legacy, &api) {
        assert_eq!(
            expected,
            crate::common::execute(&legacy, "DelayedLiteralStoreDriver")
        );
    }
}
