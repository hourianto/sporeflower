use crate::common::{checked_stdout as checked, javac as compiler};
use sporeflower::{classfile::ClassFile, ir, structure};
use std::{fs, process::Command};

#[test]
fn exceptions_from_a_catch_body_reach_the_outer_scope_not_its_siblings() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-g:none", "-d"])
            .arg(&original)
            .arg("tests/fixtures/java/SharedCatchCoverage.java"),
    );
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("SharedCatchCoverage"),
    );
    let mut class =
        ClassFile::parse(&fs::read(original.join("SharedCatchCoverage.class")).unwrap()).unwrap();
    // Obfuscators can erase optional Exceptions metadata without changing the
    // code. The outer checked catch then becomes impossible, while the inner
    // recovered catch must retain its operations and exception handling.
    class
        .methods
        .iter_mut()
        .find(|method| method.name == "quiet")
        .unwrap()
        .exceptions
        .clear();
    let classes = [class];
    let api = sporeflower::api::ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &Default::default())
        .unwrap();
    let program = sporeflower::program::Program::new(&classes, &api.classes).unwrap();
    let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
    let result =
        sporeflower::java::decompile_class(&program, &names, 0, &Default::default()).unwrap();
    assert_eq!(result.failed_methods, 0, "{:?}", result.diagnostics);
    assert!(
        !result.source.contains("catch (InterruptedException"),
        "{}",
        result.source
    );
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("SharedCatchCoverage.java"), result.source).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .arg(source.join("SharedCatchCoverage.java")),
    );
    let actual = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&rebuilt)
            .arg("SharedCatchCoverage"),
    );
    assert_eq!(actual, expected);
}

#[test]
fn shared_normal_and_exception_entries_bind_the_actual_incoming_stack() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    fs::copy(
        "tests/fixtures/shared_handler/classes/SharedExceptionalExit.class",
        original.join("SharedExceptionalExit.class"),
    )
    .unwrap();
    let support = [
        "tests/fixtures/shared_handler/EHSupport.java",
        "tests/fixtures/shared_handler/SharedExitDriver.java",
    ];
    checked(
        crate::common::compile_command()
            .args(["-classpath"])
            .arg(&original)
            .arg("-d")
            .arg(&original)
            .args(support),
    );
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("SharedExitDriver"),
    );
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(original.join("SharedExceptionalExit.class"))
            .arg(&source)
            .args(["--no-api", "--threads", "1"]),
    );
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .arg(source.join("SharedExceptionalExit.java"))
            .args(support),
    );
    let actual = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&rebuilt)
            .arg("SharedExitDriver"),
    );
    assert_eq!(
        String::from_utf8_lossy(&actual),
        String::from_utf8_lossy(&expected)
    );
}

#[test]
fn terminal_exception_scopes_preserve_shared_catches_and_finally_overrides() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-g:none", "-d"])
            .arg(&original)
            .arg("tests/fixtures/java/TerminalExceptionScopes.java"),
    );
    let expected = checked(
        Command::new(crate::common::java())
            .args(["-Xverify:all", "-cp"])
            .arg(&original)
            .arg("TerminalExceptionScopes"),
    );
    let api = work.path().join("api.jar");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let java = fs::read_to_string(source.join("TerminalExceptionScopes.java")).unwrap();
    let cleanup = java
        .split("static int cleanup(")
        .nth(1)
        .unwrap()
        .split("static int override(")
        .next()
        .unwrap();
    assert!(!cleanup.contains("label"), "{cleanup}");
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .arg(source.join("TerminalExceptionScopes.java")),
    );
    let actual = checked(
        Command::new(crate::common::java())
            .args(["-Xverify:all", "-cp"])
            .arg(&rebuilt)
            .arg("TerminalExceptionScopes"),
    );
    assert_eq!(actual, expected);
    if let Some(legacy) = crate::common::legacy_javac() {
        let legacy_output = work.path().join("legacy");
        fs::create_dir_all(&legacy_output).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&legacy_output)
                .arg(source.join("TerminalExceptionScopes.java")),
        );
        assert_eq!(
            expected,
            checked(
                Command::new(crate::common::java())
                    .args(["-Xverify:all", "-cp"])
                    .arg(&legacy_output)
                    .arg("TerminalExceptionScopes")
            )
        );
    }
}

#[test]
fn ordinary_monitor_and_catch_regions_are_structured() {
    let dir = tempfile::tempdir().unwrap();
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/java/Flow.java");
    let output = crate::common::compile_command()
        .args(["-g:none", "-d"])
        .arg(dir.path())
        .arg(source)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let class = ClassFile::parse(&fs::read(dir.path().join("Flow.class")).unwrap()).unwrap();
    for name in ["synchronizedBlock", "catchOrder"] {
        let method = class
            .methods
            .iter()
            .find(|method| method.name == name)
            .unwrap();
        let ir = ir::decompile_method(&class, method).unwrap();
        let prepared = structure::PreparedIr::new(ir);
        let tree = structure::structure(&prepared).unwrap();
        let ir = prepared.ir();
        assert_eq!(
            tree.statistics.dispatch_regions, 0,
            "{name}\nIR {ir:#?}\nTree {tree:#?}"
        );
    }
}

#[test]
fn catches_cleanup_loops_and_nested_monitors_preserve_effects_and_release() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-g:none", "-d"])
            .arg(&original)
            .arg("tests/fixtures/java/ExceptionShapes.java"),
    );
    let class =
        ClassFile::parse(&fs::read(original.join("ExceptionShapes.class")).unwrap()).unwrap();
    for name in [
        "loopCatch",
        "catchLoop",
        "retryCatch",
        "nestedMonitorDispatch",
        "finallyBranches",
    ] {
        let method = class
            .methods
            .iter()
            .find(|method| method.name == name)
            .unwrap();
        let ir = ir::decompile_method(&class, method).unwrap();
        let prepared = structure::PreparedIr::new(ir);
        let tree = structure::structure(&prepared).unwrap();
        assert_eq!(
            tree.statistics.dispatch_regions, 0,
            "ordinary catch flow needs no dispatcher: {name}\n{tree:#?}"
        );
    }
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("ExceptionShapes"),
    );
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .args(["--no-api", "--threads", "1"]),
    );
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .arg(source.join("ExceptionShapes.java")),
    );
    let actual = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&rebuilt)
            .arg("ExceptionShapes"),
    );
    assert_eq!(
        String::from_utf8_lossy(&actual),
        String::from_utf8_lossy(&expected)
    );
}

#[test]
fn legacy_handler_priority_and_cleanup_continuations_preserve_behavior() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(original.join("pkg")).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    for entry in fs::read_dir("tests/fixtures/exception/classes/pkg").unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), original.join("pkg").join(entry.file_name())).unwrap();
    }
    let support = [
        "tests/fixtures/exception/FinallyReturnState.java",
        "tests/fixtures/exception/ExceptionDispatchState.java",
        "tests/fixtures/java/ExceptionBytecodeDriver.java",
    ];
    checked(
        crate::common::compile_command()
            .args(["-g:none", "-d"])
            .arg(&original)
            .arg("-cp")
            .arg(&original)
            .args(support),
    );
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("ExceptionBytecodeDriver"),
    );
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg("tests/fixtures/exception/classes")
            .arg(&source)
            .args(["--no-api", "--threads", "1"]),
    );
    let mut command = Command::new(compiler());
    command
        .args(["-source", "7", "-target", "7", "-d"])
        .arg(&rebuilt)
        .args(support);
    for entry in fs::read_dir(source.join("pkg")).unwrap() {
        command.arg(entry.unwrap().path());
    }
    checked(&mut command);
    let actual = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&rebuilt)
            .arg("ExceptionBytecodeDriver"),
    );
    assert_eq!(
        String::from_utf8_lossy(&actual),
        String::from_utf8_lossy(&expected)
    );
}
