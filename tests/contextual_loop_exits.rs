use crate::common::checked_stdout as checked;
use sporeflower::{classfile::ClassFile, ir, structure};
use std::{fs, process::Command};

#[test]
fn loop_exit_corridors_keep_guard_effects_and_enclosing_transfers() {
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
            .arg("tests/fixtures/java/ContextualLoopExits.java"),
    );
    let class =
        ClassFile::parse(&fs::read(original.join("ContextualLoopExits.class")).unwrap()).unwrap();
    for name in ["scan", "restart", "nestedBreaks", "selectedDecision"] {
        let method = class
            .methods
            .iter()
            .find(|method| method.name == name)
            .unwrap();
        let ir = ir::decompile_method(&class, method).unwrap();
        let prepared = structure::PreparedIr::new(ir);
        let tree = structure::structure(&prepared).unwrap();
        assert_eq!(tree.statistics.labeled_regions, 0, "{name}: {tree:#?}");
        assert_eq!(tree.statistics.dispatch_regions, 0, "{name}: {tree:#?}");
    }
    let expected = checked(
        Command::new(crate::common::java())
            .args(["-Xverify:all", "-cp"])
            .arg(&original)
            .arg("ContextualLoopExits"),
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
    let java = source.join("ContextualLoopExits.java");
    let rendered = fs::read_to_string(&java).unwrap();
    let selected = rendered
        .split(" selectedDecision(")
        .nth(1)
        .unwrap()
        .split("\n    }")
        .next()
        .unwrap();
    assert!(selected.contains('?'), "{selected}");
    assert_eq!(selected.matches("mark(50)").count(), 1, "{selected}");
    for (name, selected) in [
        ("redundantDecision", false),
        ("volatileDecision", true),
        ("effectfulDecision", true),
    ] {
        let body = rendered
            .split(&format!(" {name}("))
            .nth(1)
            .unwrap()
            .split("\n    }")
            .next()
            .unwrap();
        assert_eq!(body.contains('?'), selected, "{name}: {body}");
    }
    for name in ["scan", "nestedBreaks"] {
        let start = rendered
            .find(&format!("public static int {name}("))
            .unwrap();
        let end = rendered[start..].find("\n    }").unwrap() + start;
        assert!(
            !rendered[start..end].contains("label"),
            "{}",
            &rendered[start..end]
        );
    }
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-d"])
            .arg(&rebuilt)
            .arg(&java),
    );
    assert_eq!(
        expected,
        checked(
            Command::new(crate::common::java())
                .args(["-Xverify:all", "-cp"])
                .arg(&rebuilt)
                .arg("ContextualLoopExits")
        )
    );

    if let Some(legacy) = crate::common::legacy_javac() {
        let output = work.path().join("legacy");
        fs::create_dir_all(&output).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&output)
                .arg(&java),
        );
        assert_eq!(
            expected,
            checked(
                Command::new(crate::common::java())
                    .args(["-Xverify:all", "-cp"])
                    .arg(&output)
                    .arg("ContextualLoopExits")
            )
        );
    }
}
