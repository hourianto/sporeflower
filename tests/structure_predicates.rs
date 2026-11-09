use crate::common::checked_stdout as checked;
use sporeflower::{
    classfile::ClassFile,
    ir,
    structure::{self, Predicate, Stmt},
};
use std::{fs, process::Command};

#[test]
fn two_arm_layout_preserves_bytecode_order_and_predicate_effects() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    let api = work.path().join("api.jar");
    fs::create_dir_all(&original).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-g:none", "-d"])
            .arg(&original)
            .arg("tests/fixtures/java/BranchLayout.java"),
    );
    let expected = crate::common::execute(&original, "BranchLayout");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let output = fs::read_to_string(source.join("BranchLayout.java")).unwrap();
    let method = |name| crate::common::source::method(&output, name);
    let two_arm = method("twoArm");
    assert!(two_arm.contains("if (p0 == 5)"), "{two_arm}");
    assert!(
        two_arm.find("hit(41)").unwrap() < two_arm.find("hit(43)").unwrap(),
        "{two_arm}"
    );
    let negated = method("negated");
    assert!(negated.contains("if (p0 != 5)"), "{negated}");
    let nested_then = method("nestedThen");
    assert!(nested_then.contains("if (p0 == 1)"), "{nested_then}");
    assert!(nested_then.contains("if (p1 == 2)"), "{nested_then}");
    assert!(!nested_then.contains("else if"), "{nested_then}");
    assert!(
        nested_then.find("hit(151)").unwrap() < nested_then.find("hit(157)").unwrap(),
        "{nested_then}"
    );
    let nested = method("nestedDispatch");
    for state in 2..=4 {
        assert!(
            nested.contains(&format!("}} else if (p0 == {state})")),
            "{nested}"
        );
    }
    assert!(!nested.contains("p0 !="), "{nested}");
    let guard = method("guard");
    assert!(
        guard.contains("throw new IllegalArgumentException()"),
        "{guard}"
    );
    assert!(!guard.contains("else"), "{guard}");
    let typed = method("typedDispatch");
    assert!(typed.contains("} else if (p0 instanceof Long)"), "{typed}");
    assert!(!typed.contains("!(p0 instanceof Long)"), "{typed}");
    crate::common::compile(&crate::common::sources(&source), &rebuilt, None);
    assert_eq!(expected, crate::common::execute(&rebuilt, "BranchLayout"));
    let legacy = work.path().join("legacy");
    if crate::common::legacy_compile(&crate::common::sources(&source), &legacy, &api) {
        assert_eq!(expected, crate::common::execute(&legacy, "BranchLayout"));
    }
}

#[test]
fn compound_tests_preserve_short_circuit_effects_and_remove_forward_label_ladders() {
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
            .arg("tests/fixtures/java/CompoundConditions.java"),
    );
    let class =
        ClassFile::parse(&fs::read(original.join("CompoundConditions.class")).unwrap()).unwrap();
    let method = class
        .methods
        .iter()
        .find(|method| method.name == "key")
        .unwrap();
    let ir = ir::decompile_method(&class, method).unwrap();
    let prepared = structure::PreparedIr::new(ir);
    let tree = structure::structure(&prepared).unwrap();
    assert_eq!(tree.statistics.labeled_regions, 0, "{tree:#?}");
    assert!(tree.body.iter().any(|statement| matches!(statement, Stmt::If { condition: Predicate::Or(terms), .. } if terms.len() == 2)), "{tree:#?}");
    let shared = class
        .methods
        .iter()
        .find(|method| method.name == "sharedBody")
        .unwrap();
    let shared_ir = ir::decompile_method(&class, shared).unwrap();
    let shared_prepared = structure::PreparedIr::new(shared_ir);
    let shared_tree = structure::structure(&shared_prepared).unwrap();
    assert_eq!(
        shared_tree.statistics.labeled_regions, 0,
        "{shared_tree:#?}"
    );
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("CompoundConditions"),
    );
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .args(["--no-api", "--threads", "1"]),
    );
    let output = fs::read_to_string(source.join("CompoundConditions.java")).unwrap();
    assert!(output.contains(" || "), "{output}");
    assert!(output.contains(" && "), "{output}");
    let categorical = output
        .split(" categorical(")
        .nth(1)
        .unwrap()
        .split("\n    }")
        .next()
        .unwrap();
    assert_eq!(categorical.matches("else if (").count(), 2, "{categorical}");
    let nested = output
        .split(" nestedDecisionBody(")
        .nth(1)
        .unwrap()
        .split("\n    }")
        .next()
        .unwrap();
    assert!(!nested.contains("label"), "{nested}");
    assert!(
        nested.contains(" || ") && nested.contains(" && "),
        "{nested}"
    );
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .arg(source.join("CompoundConditions.java")),
    );
    let actual = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&rebuilt)
            .arg("CompoundConditions"),
    );
    assert_eq!(actual, expected);
}
