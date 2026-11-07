use crate::common::checked_stdout as checked;
use std::{fs, path::Path, process::Command};

#[test]
fn value_exits_preserve_catch_cleanup_monitor_and_loop_observations() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-g:none", "-d"])
            .arg(&original)
            .arg("tests/fixtures/java/ValueContinuations.java"),
    );
    let class = sporeflower::classfile::ClassFile::parse(
        &fs::read(original.join("ValueContinuations.class")).unwrap(),
    )
    .unwrap();
    for name in ["caughtResult", "monitoredResult"] {
        let method = class
            .methods
            .iter()
            .find(|method| method.name == name)
            .unwrap();
        let ir = sporeflower::ir::decompile_method(&class, method).unwrap();
        let (value, definition) = ir
            .values
            .iter_enumerated()
            .find(|(_, value)| {
                matches!(
                    value.kind,
                    sporeflower::ir::ValueKind::Literal(sporeflower::ir::Literal::Int(1))
                )
            })
            .unwrap();
        assert!(ir.blocks[definition.block].statements.iter().any(|statement|
            matches!(statement, sporeflower::ir::Statement::Assign { value: assigned, .. } if assigned.value() == value)
        ), "{name}: the original true store must retain its definition before cleanup");
    }
    let run = |classes: &Path| {
        checked(
            Command::new(crate::common::java())
                .args(["-Xverify:all", "-cp"])
                .arg(classes)
                .arg("ValueContinuations"),
        )
    };
    let expected = run(&original);
    let api = work.path().join("api.jar");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let java = source.join("ValueContinuations.java");
    let rendered = fs::read_to_string(&java).unwrap();
    assert!(!rendered.contains("$dispatch"), "{rendered}");
    for (method, labels) in [("caughtResult", 0), ("monitoredResult", 0), ("search", 1)] {
        let start = rendered
            .find(&format!("public static int {method}("))
            .unwrap();
        let end = rendered[start..].find("\n    }").unwrap() + start;
        assert!(
            rendered[start..end].matches(": {").count() <= labels,
            "{}",
            &rendered[start..end]
        );
    }
    let start = rendered.find("public static int nestedRows(").unwrap();
    let end = rendered[start..].find("\n    }").unwrap() + start;
    assert!(
        !rendered[start..end].contains("while (true)"),
        "{}",
        &rendered[start..end]
    );
    assert!(
        !rendered[start..end].contains("continue label"),
        "{}",
        &rendered[start..end]
    );
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-d"])
            .arg(&rebuilt)
            .arg(&java),
    );
    assert_eq!(expected, run(&rebuilt), "{rendered}");
    if let Some(legacy) = crate::common::legacy_javac() {
        let classes = work.path().join("legacy");
        fs::create_dir_all(&classes).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&classes)
                .arg(&java),
        );
        assert_eq!(expected, run(&classes), "{rendered}");
    }
}
