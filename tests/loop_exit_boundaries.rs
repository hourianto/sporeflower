use crate::common::checked_stdout as checked;
use sporeflower::{
    classfile::ClassFile,
    ir,
    structure::{self, Stmt},
};
use std::{fs, process::Command};

#[test]
fn preserved_constant_loop_guards_keep_the_body_and_continuation_compilable() {
    use sporeflower::{
        java::{JavaOptions, decompile_class},
        naming::NamePlan,
        program::Program,
    };

    let mut class = ClassFile::parse(include_bytes!(
        "fixtures/legacy/classes/pkg/TestConsecutiveReads.class"
    ))
    .unwrap();
    class.name = "LiteralLoopGuard".into();
    class.fields.clear();
    let template = class.methods.iter().find(|m| m.code.is_some()).unwrap();
    class.methods = [("never", 0x03), ("forever", 0x04)]
        .into_iter()
        .map(|(name, iconst)| {
            let mut method = template.clone();
            method.name = name.into();
            method.access_flags = 0x0009;
            method.descriptor = "(I)I".into();
            let code = method.code.as_mut().unwrap();
            // while (literal != 0) { ++arg; } return arg;
            code.bytes = vec![iconst, 0x99, 0, 9, 0x84, 0, 1, 0xa7, 0xff, 0xf9, 0x1a, 0xac];
            code.max_stack = 1;
            code.max_locals = 1;
            code.exception_table.clear();
            code.local_variables.clear();
            method
        })
        .collect();
    let classes = [class];
    let program = Program::new(&classes, &[]).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let options = JavaOptions {
        preserve_bytecode: true,
        ..Default::default()
    };
    let output = decompile_class(&program, &names, 0, &options).unwrap();
    assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
    for name in ["never", "forever"] {
        let body = crate::common::source::method(&output.source, name);
        assert!(
            body.contains("break;") && body.contains("return "),
            "{body}"
        );
    }
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("LiteralLoopGuard.java");
    fs::write(&source, output.source).unwrap();
    crate::common::compile(&[source], work.path(), None);
}

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
