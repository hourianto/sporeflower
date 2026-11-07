//! Null-bottom storage typing must preserve descriptor-based overload binding.
use crate::common::checked;
use sporeflower::classfile::{ClassFile, JvmType};
use sporeflower::ir::{self, ValueKind};
use std::{fs, path::Path, process::Command};

fn execute(classes: &Path) -> Vec<u8> {
    checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(classes)
            .arg("NullStorageTypes"),
    )
    .stdout
}

#[test]
fn null_loop_storage_preserves_overloads_and_exception_observations() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    for path in [&original, &rebuilt] {
        fs::create_dir_all(path).unwrap();
    }
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&original)
            .arg("tests/fixtures/java/NullStorageTypes.java"),
    );
    let class =
        ClassFile::parse(&fs::read(original.join("NullStorageTypes.class")).unwrap()).unwrap();
    let method = class
        .methods
        .iter()
        .find(|method| method.name == "run")
        .unwrap();
    let ir = ir::decompile_method(&class, method).unwrap();
    assert!(
        ir.values.iter().any(|value| {
            matches!(value.kind, ValueKind::Phi { .. })
                && value.ty == JvmType::Object("java/lang/Object".into())
                && value.local.is_some_and(|local| {
                    ir.locals[local].ty == JvmType::Object("NullStorageTypes$Resource".into())
                })
        }),
        "fixture must refine an Object-typed null phi into the resource storage type"
    );
    let expected = execute(&original);
    assert_eq!(expected.iter().filter(|&&byte| byte == b'\n').count(), 55);
    let api = work.path().join("api.jar");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let sources: Vec<_> = fs::read_dir(&source)
        .unwrap()
        .map(Result::unwrap)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "java")
        })
        .collect();
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .args(&sources),
    );
    assert_eq!(execute(&rebuilt), expected);
    if let Some(legacy) = crate::common::legacy_javac() {
        let output = work.path().join("legacy");
        fs::create_dir_all(&output).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&output)
                .args(&sources),
        );
        assert_eq!(execute(&output), expected);
    }
}
