use crate::common::checked_stdout as checked;
use sporeflower::{
    api::{ApiCatalog, ApiOptions},
    classfile::ClassFile,
    naming::NamePlan,
    program::Program,
};
use std::{fs, process::Command};

#[test]
fn returning_cleanup_preserves_each_result_and_cleanup_exception_precedence() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-g:none", "-d"])
            .arg(&original)
            .arg("tests/fixtures/java/ReturningCleanup.java"),
    );
    let class_path = original.join("ReturningCleanup.class");
    let mut bytes = fs::read(&class_path).unwrap();
    let parsed = ClassFile::parse(&bytes).unwrap();
    let code = parsed
        .methods
        .iter()
        .find(|method| method.name == "external")
        .unwrap()
        .code
        .as_ref()
        .unwrap();
    let decoded = sporeflower::bytecode::decode(&code.bytes).unwrap();
    let branch = decoded
        .iter()
        .find(|instruction| instruction.is_conditional())
        .unwrap();
    let log = decoded
        .iter()
        .find(|instruction| {
            instruction.opcode == 0xb8
                && matches!(instruction.operand, sporeflower::bytecode::Operand::Constant(index)
            if parsed.constant_pool.member_ref(index).unwrap().name == "log")
        })
        .unwrap();
    let locations: Vec<_> = bytes
        .windows(code.bytes.len())
        .enumerate()
        .filter_map(|(at, code_bytes)| (code_bytes == code.bytes).then_some(at))
        .collect();
    assert_eq!(locations.len(), 1);
    let offset = locations[0] + branch.pc as usize + 1;
    // The outside edge enters after close(), in the middle of the copied
    // returning cleanup. Factoring must not add the skipped close to that edge.
    bytes[offset..offset + 2]
        .copy_from_slice(&((log.pc as i32 - branch.pc as i32) as i16).to_be_bytes());
    bytes[4..8].copy_from_slice(&[0, 3, 0, 45]);
    fs::write(&class_path, &bytes).unwrap();
    let expected = checked(
        Command::new(crate::common::java())
            .args(["-Xverify:all", "-cp"])
            .arg(&original)
            .arg("ReturningCleanup"),
    );
    let classes = vec![ClassFile::parse(&bytes).unwrap()];
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
    let source = rebuilt.join("ReturningCleanup.java");
    fs::write(&source, &output.source).unwrap();
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-d"])
            .arg(&rebuilt)
            .arg(&source),
    );
    assert_eq!(
        expected,
        checked(
            Command::new(crate::common::java())
                .args(["-Xverify:all", "-cp"])
                .arg(&rebuilt)
                .arg("ReturningCleanup")
        ),
        "{}",
        output.source
    );
    if let Some(compiler) = crate::common::legacy_javac() {
        let stubs = work.path().join("api.jar");
        api.write_jar(&stubs).unwrap();
        checked(
            crate::common::legacy_command(compiler, &stubs)
                .arg("-d")
                .arg(&rebuilt)
                .arg(&source),
        );
        assert_eq!(
            expected,
            checked(
                Command::new(crate::common::java())
                    .args(["-Xverify:all", "-cp"])
                    .arg(&rebuilt)
                    .arg("ReturningCleanup")
            ),
            "{}",
            output.source
        );
    }
    for name in ["returning", "constant", "simple"] {
        let start = output.source.find(&format!("String {name}(")).unwrap();
        let open = start + output.source[start..].find('{').unwrap();
        let mut depth = 1;
        let mut end = open + 1;
        while depth != 0 {
            match output.source.as_bytes()[end] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => (),
            }
            end += 1;
        }
        assert_eq!(
            output.source[start..end].matches("finally {").count(),
            1,
            "{}",
            &output.source[start..end]
        );
    }
}
