use crate::common::checked_stdout as checked;
use sporeflower::{
    classfile::ClassFile,
    ir,
    structure::{self, Stmt},
};
use std::{fs, process::Command};

#[test]
fn shared_region_tails_keep_case_bodies_and_exception_boundaries() {
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
            .arg("tests/fixtures/java/RegionBoundaries.java"),
    );
    let class =
        ClassFile::parse(&fs::read(original.join("RegionBoundaries.class")).unwrap()).unwrap();
    let method = class
        .methods
        .iter()
        .find(|method| method.name == "caseTail")
        .unwrap();
    let ir = ir::decompile_method(&class, method).unwrap();
    let prepared = structure::PreparedIr::new(ir);
    let tree = structure::structure(&prepared).unwrap();
    assert!(
        tree.body
            .iter()
            .any(|statement| matches!(statement, Stmt::Switch { cases, .. } if cases.len() == 3)),
        "{tree:#?}"
    );
    assert!(
        !tree
            .body
            .iter()
            .any(|statement| matches!(statement, Stmt::Label { .. })),
        "{tree:#?}"
    );
    let expected = checked(
        Command::new(crate::common::java())
            .args(["-Xverify:all", "-cp"])
            .arg(&original)
            .arg("RegionBoundaries"),
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
    let java = source.join("RegionBoundaries.java");
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
                .arg("RegionBoundaries")
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
                    .arg("RegionBoundaries")
            )
        );
    }
}
