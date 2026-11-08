use crate::common::checked_stdout as checked;
use sporeflower::{
    classfile::{ClassFile, Code, Method},
    ir::Member,
    program::Program,
};
use std::{fs, process::Command};

#[test]
fn integer_result_certificate_rejects_alternate_return_entries_and_noncanonical_values() {
    let mut class = ClassFile::parse(include_bytes!(
        "fixtures/legacy/classes/pkg/TestConsecutiveReads.class"
    ))
    .unwrap();
    let template = class
        .methods
        .iter()
        .find(|method| method.code.is_some())
        .unwrap()
        .clone();
    let mut methods = Vec::new();
    for (name, descriptor, bytes) in [
        ("zero", "()Z", vec![0x03, 0xac]),
        ("noncanonical", "()Z", vec![0x05, 0xac]),
        (
            "multiple",
            "(I)I",
            vec![0x1a, 0x99, 0x00, 0x05, 0x07, 0xac, 0x07, 0xac],
        ),
        (
            "different",
            "(I)I",
            vec![0x1a, 0x99, 0x00, 0x05, 0x07, 0xac, 0x08, 0xac],
        ),
        // ICONST_1; GOTO return, bypassing the physically preceding ICONST_0.
        (
            "enteredReturn",
            "()I",
            vec![0x04, 0xa7, 0x00, 0x04, 0x03, 0xac],
        ),
        (
            "joinedPredicate",
            "(I)Z",
            vec![0x1a, 0x99, 0x00, 0x07, 0x04, 0xa7, 0x00, 0x04, 0x03, 0xac],
        ),
        (
            "joinedRaw",
            "(I)Z",
            vec![0x1a, 0x99, 0x00, 0x07, 0x04, 0xa7, 0x00, 0x04, 0x1a, 0xac],
        ),
    ] {
        let mut method: Method = template.clone();
        method.name = name.into();
        method.descriptor = descriptor.into();
        method.access_flags = 9;
        let code: &mut Code = method.code.as_mut().unwrap();
        code.bytes = bytes;
        code.max_stack = 1;
        code.max_locals = 1;
        code.exception_table.clear();
        code.local_variables.clear();
        methods.push(method);
    }
    class.methods = methods;
    let classes = vec![class];
    let program = Program::new(&classes, &[]).unwrap();
    for (name, descriptor, expected) in [
        ("zero", "()Z", Some(0)),
        ("noncanonical", "()Z", None),
        ("multiple", "(I)I", Some(4)),
        ("different", "(I)I", None),
        ("enteredReturn", "()I", None),
        ("joinedPredicate", "(I)Z", None),
        ("joinedRaw", "(I)Z", None),
    ] {
        assert_eq!(
            program.normal_int_return(&Member {
                owner: classes[0].name.clone(),
                name: name.into(),
                descriptor: descriptor.into()
            }),
            expected,
            "{name}"
        );
    }
}

#[test]
fn certified_results_keep_invocation_order_initialization_exceptions_and_virtual_dispatch() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&original)
            .arg("tests/fixtures/java/ConstantReturns.java"),
    );
    let expected = checked(
        Command::new(crate::common::java())
            .args(["-Xverify:all", "-cp"])
            .arg(&original)
            .arg("ConstantReturns"),
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
    let text = fs::read_to_string(source.join("ConstantReturns.java")).unwrap();
    for dead in ["1001", "1002", "1003", "1004"] {
        assert!(!text.contains(dead), "{text}");
    }
    let java: Vec<_> = fs::read_dir(&source)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "java"))
        .collect();
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .args(&java),
    );
    assert_eq!(
        expected,
        checked(
            Command::new(crate::common::java())
                .args(["-Xverify:all", "-cp"])
                .arg(&rebuilt)
                .arg("ConstantReturns")
        )
    );
    if let Some(legacy) = crate::common::legacy_javac() {
        let out = work.path().join("legacy");
        fs::create_dir(&out).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&out)
                .args(&java),
        );
        assert_eq!(
            expected,
            checked(
                Command::new(crate::common::java())
                    .args(["-Xverify:all", "-cp"])
                    .arg(&out)
                    .arg("ConstantReturns")
            )
        );
    }
}
