use crate::common::checked_stdout as checked;
use sporeflower::{
    classfile::ClassFile,
    ir,
    structure::{self, Stmt},
};
use std::{fs, process::Command};

#[test]
fn loop_guards_preserve_return_evaluation_updates_and_cleanup() {
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
            .arg("tests/fixtures/java/LoopExitBoundaries.java"),
    );
    let class =
        ClassFile::parse(&fs::read(original.join("LoopExitBoundaries.class")).unwrap()).unwrap();
    let method = class
        .methods
        .iter()
        .find(|method| method.name == "empty")
        .unwrap();
    let ir = ir::decompile_method(&class, method).unwrap();
    let prepared = structure::PreparedIr::new(ir);
    let tree = structure::structure(&prepared).unwrap();
    let (region, body) = tree
        .body
        .iter()
        .find_map(|statement| match statement {
            Stmt::Loop { region, body, .. } => Some((*region, body)),
            _ => None,
        })
        .expect("empty scanning loop");
    assert!(
        matches!(body.as_slice(), [Stmt::If { then_body, else_body, .. }]
        if else_body.is_empty() && matches!(then_body.as_slice(), [Stmt::Break { target }] if *target == region)),
        "{tree:#?}"
    );
    let expected = checked(
        Command::new(crate::common::java())
            .args(["-Xverify:all", "-cp"])
            .arg(&original)
            .arg("LoopExitBoundaries"),
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
    let java = source.join("LoopExitBoundaries.java");
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
                .arg("LoopExitBoundaries")
        )
    );

    if let Some(legacy) = crate::common::legacy_javac() {
        let legacy_output = work.path().join("legacy");
        fs::create_dir_all(&legacy_output).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&legacy_output)
                .arg(&java),
        );
        assert_eq!(
            expected,
            checked(
                Command::new(crate::common::java())
                    .args(["-Xverify:all", "-cp"])
                    .arg(&legacy_output)
                    .arg("LoopExitBoundaries")
            )
        );
    }
}
