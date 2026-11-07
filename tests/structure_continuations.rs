use crate::common::checked_stdout as checked;
use sporeflower::{
    classfile::ClassFile,
    ir,
    structure::{self, Stmt},
};
use std::{fs, process::Command};

fn continuations(body: &[Stmt]) -> usize {
    body.iter()
        .map(|statement| match statement {
            Stmt::Loop {
                body, continuation, ..
            } => usize::from(continuation.is_some()) + continuations(body),
            Stmt::If {
                then_body,
                else_body,
                ..
            } => continuations(then_body) + continuations(else_body),
            Stmt::Label { body, .. } | Stmt::Synchronized { body, .. } => continuations(body),
            Stmt::Switch { cases, .. } => cases.iter().map(|case| continuations(&case.body)).sum(),
            Stmt::Try { body, catches } => {
                continuations(body)
                    + catches
                        .iter()
                        .map(|catch| continuations(&catch.body))
                        .sum::<usize>()
            }
            Stmt::TryFinally { body, cleanup, .. } => continuations(body) + continuations(cleanup),
            _ => 0,
        })
        .sum()
}

#[test]
fn shared_tails_and_nested_continues_preserve_update_and_cleanup_counts() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-g:none", "-d"])
            .arg(&original)
            .arg("tests/fixtures/java/OwnedLoopContinuation.java"),
    );
    let class =
        ClassFile::parse(&fs::read(original.join("OwnedLoopContinuation.class")).unwrap()).unwrap();
    for (name, expected) in [
        ("sharedTail", true),
        ("directHeader", false),
        ("optionalSwitchArm", true),
    ] {
        let method = class
            .methods
            .iter()
            .find(|method| method.name == name)
            .unwrap();
        let ir = ir::decompile_method(&class, method).unwrap();
        let prepared = structure::PreparedIr::new(ir);
        let tree = structure::structure(&prepared).unwrap();
        assert_eq!(continuations(&tree.body) > 0, expected, "{name}: {tree:#?}");
        if expected {
            assert_eq!(tree.statistics.labeled_regions, 0, "{tree:#?}");
        }
    }
    let expected = checked(
        Command::new(crate::common::java())
            .args(["-Xverify:all", "-cp"])
            .arg(&original)
            .arg("OwnedLoopContinuation"),
    );
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .args(["--no-api", "--threads", "1"]),
    );
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .arg(source.join("OwnedLoopContinuation.java")),
    );
    let actual = checked(
        Command::new(crate::common::java())
            .args(["-Xverify:all", "-cp"])
            .arg(&rebuilt)
            .arg("OwnedLoopContinuation"),
    );
    assert_eq!(actual, expected);
}
