//! Case-edge literal placement should expose source switch fallthrough while
//! preserving selector effects, old values observed by handlers, and loop reset.
use crate::common::checked_stdout as checked;
use sporeflower::{
    classfile::ClassFile,
    ir,
    structure::{self, Stmt},
};
use std::{fs, path::Path, process::Command};

fn has_fallthrough(body: &[Stmt]) -> bool {
    body.iter().any(|statement| match statement {
        Stmt::Switch { cases, .. } => cases.windows(2).any(|pair| {
            pair[0].keys.contains(&32)
                && pair[1].keys.contains(&17)
                && pair[0]
                    .body
                    .iter()
                    .all(|statement| matches!(statement, Stmt::Basic(_)))
        }),
        Stmt::Loop { body, .. } | Stmt::Label { body, .. } => has_fallthrough(body),
        _ => false,
    })
}

fn execute(classes: &Path) -> Vec<u8> {
    checked(
        Command::new(crate::common::java())
            .args(["-Xverify:all", "-cp"])
            .arg(classes)
            .arg("SwitchDefaults"),
    )
}

#[test]
fn literal_default_exposes_fallthrough_and_preserves_effects() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    for path in [&original, &rebuilt] {
        fs::create_dir_all(path).unwrap();
    }
    checked(
        crate::common::compile_command()
            .args(["-g:none", "-d"])
            .arg(&original)
            .arg("tests/fixtures/java/SwitchDefaults.java"),
    );
    let class =
        ClassFile::parse(&fs::read(original.join("SwitchDefaults.class")).unwrap()).unwrap();
    for name in ["simple", "repeated"] {
        let method = class
            .methods
            .iter()
            .find(|method| method.name == name)
            .unwrap();
        let ir = ir::decompile_method(&class, method).unwrap();
        let prepared = structure::PreparedIr::new(ir);
        let tree = structure::structure(&prepared).unwrap();
        assert!(has_fallthrough(&tree.body), "{name}: {tree:#?}");
        assert_eq!(tree.statistics.labeled_regions, 0, "{name}: {tree:#?}");
    }
    let expected = execute(&original);
    assert_eq!(expected.iter().filter(|&&byte| byte == b'\n').count(), 92);
    let api = work.path().join("api.jar");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let java = source.join("SwitchDefaults.java");
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .arg(&java),
    );
    assert_eq!(execute(&rebuilt), expected);
    if let Some(legacy) = crate::common::legacy_javac() {
        let output = work.path().join("legacy");
        fs::create_dir_all(&output).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&output)
                .arg(&java),
        );
        assert_eq!(execute(&output), expected);
    }
}
