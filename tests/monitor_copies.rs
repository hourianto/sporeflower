use crate::common::{checked, javac};
use std::{fs, path::Path, process::Command};

#[test]
fn monitor_cleanup_rethrows_the_exception_from_its_actual_entry() {
    let cleared = sporeflower::classfile::ClassFile::parse(
        &fs::read("tests/fixtures/monitor_copies/classes/MonitorCopiesCleared.class").unwrap(),
    )
    .unwrap();
    let ir = sporeflower::ir::decompile_method(&cleared, &cleared.methods[0]).unwrap();
    assert!(
        sporeflower::structure::structure(&sporeflower::structure::PreparedIr::new(ir))
            .unwrap()
            .suppressed_monitors
            .is_empty(),
        "replacing the caught exception with null must not become an implicit same-exception cleanup"
    );
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let generated = work.path().join("generated");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    let class = Path::new("tests/fixtures/monitor_copies/classes/MonitorCopies.class");
    fs::copy(class, original.join("MonitorCopies.class")).unwrap();
    let support = [
        "tests/fixtures/shared_handler/EHSupport.java",
        "tests/fixtures/monitor_copies/MonitorCopiesDriver.java",
    ];
    checked(
        crate::common::compile_command()
            .args(["-classpath"])
            .arg(&original)
            .arg("-d")
            .arg(&original)
            .args(support),
    );
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(class)
            .arg(&generated)
            .args(["--no-api", "--threads", "1", "--classpath"])
            .arg(&original),
    );
    let source = generated.join("MonitorCopies.java");
    assert!(
        fs::read_to_string(&source)
            .unwrap()
            .contains("synchronized (")
    );
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .arg(source)
            .args(support),
    );
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("MonitorCopiesDriver"),
    )
    .stdout;
    let actual = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&rebuilt)
            .arg("MonitorCopiesDriver"),
    )
    .stdout;
    assert_eq!(expected, actual);
    assert!(
        String::from_utf8_lossy(&actual)
            .lines()
            .all(|line| line.ends_with(":false")),
        "every normal and exceptional path must release the monitor"
    );
}

#[test]
fn finally_composes_proved_monitor_regions_without_changing_lock_ownership() {
    use sporeflower::{
        api::{ApiCatalog, ApiOptions},
        classfile::ClassFile,
        naming::NamePlan,
        program::Program,
        structure::PreparedIr,
    };
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    checked(
        Command::new(javac())
            .args(["--release", "8", "-g:none", "-d"])
            .arg(&original)
            .arg("tests/fixtures/java/MonitorCleanup.java"),
    );
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-Xverify:all")
            .arg("-cp")
            .arg(&original)
            .arg("MonitorCleanup"),
    )
    .stdout;
    let classes =
        vec![ClassFile::parse(&fs::read(original.join("MonitorCleanup.class")).unwrap()).unwrap()];
    for name in [
        "nested",
        "conditional",
        "differentLocks",
        "cleanupBeforeMonitor",
    ] {
        let method = classes[0]
            .methods
            .iter()
            .find(|method| method.name == name)
            .unwrap();
        let ir = PreparedIr::new(sporeflower::ir::decompile_method(&classes[0], method).unwrap());
        let tree = sporeflower::structure::structure(&ir).unwrap();
        assert_eq!(
            format!("{tree:?}").matches("TryFinally {").count(),
            usize::from(name != "differentLocks"),
            "{name}: {tree:#?}"
        );
        if name == "cleanupBeforeMonitor" {
            assert_eq!(
                format!("{tree:?}").matches("Synchronized {").count(),
                2,
                "the retained continuation owns each nested monitor once: {tree:#?}"
            );
        }
    }
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let output =
        sporeflower::java::decompile_class(&program, &names, 0, &Default::default()).unwrap();
    assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
    assert!(!output.source.contains("$dispatch"), "{}", output.source);
    let source = rebuilt.join("MonitorCleanup.java");
    fs::write(&source, &output.source).unwrap();
    fs::copy(
        original.join("MonitorCleanupProbe.class"),
        rebuilt.join("MonitorCleanupProbe.class"),
    )
    .unwrap();
    checked(
        Command::new(javac())
            .args(["--release", "8", "-classpath"])
            .arg(&original)
            .arg("-d")
            .arg(&rebuilt)
            .arg(&source),
    );
    assert_eq!(
        expected,
        checked(
            Command::new(crate::common::java())
                .arg("-Xverify:all")
                .arg("-cp")
                .arg(&rebuilt)
                .arg("MonitorCleanup")
        )
        .stdout,
        "{}",
        output.source
    );
    if let Some(compiler) = crate::common::legacy_javac() {
        let stubs = work.path().join("api.jar");
        api.write_jar(&stubs).unwrap();
        // holdsLock is a JVM-side observation outside CLDC. Supply its helper
        // declaration to the old compiler, then retain the real implementation
        // for both executions.
        let probe = work.path().join("MonitorCleanupProbe.java");
        fs::write(
            &probe,
            "class MonitorCleanupProbe { public static native boolean held(Object lock); }",
        )
        .unwrap();
        checked(
            crate::common::legacy_command(compiler, &stubs)
                .arg("-d")
                .arg(&rebuilt)
                .arg(&source)
                .arg(&probe),
        );
        fs::copy(
            original.join("MonitorCleanupProbe.class"),
            rebuilt.join("MonitorCleanupProbe.class"),
        )
        .unwrap();
        assert_eq!(
            expected,
            checked(
                Command::new(crate::common::java())
                    .arg("-Xverify:all")
                    .arg("-cp")
                    .arg(&rebuilt)
                    .arg("MonitorCleanup")
            )
            .stdout,
            "{}",
            output.source
        );
    }
}
