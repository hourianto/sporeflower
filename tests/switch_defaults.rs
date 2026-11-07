//! Case-edge literal placement should expose source switch fallthrough while
//! preserving selector effects, old values observed by handlers, and loop reset.
//! Exact default destinations never need explicit source case labels.
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

#[test]
fn exact_default_entries_disappear_before_source_recovery() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    crate::common::compile(
        &["tests/fixtures/java/SwitchDefaultTargets.java".into()],
        &original,
        None,
    );
    let class =
        ClassFile::parse(&fs::read(original.join("SwitchDefaultTargets.class")).unwrap()).unwrap();
    for name in ["dense", "sparse", "allDefault", "emptyDefault", "cleanup"] {
        let method = class
            .methods
            .iter()
            .find(|method| method.name == name)
            .unwrap();
        let decoded = sporeflower::bytecode::decode(&method.code.as_ref().unwrap().bytes).unwrap();
        if let Some(opcode) = match name {
            "dense" | "cleanup" => Some(0xaa),
            "sparse" => Some(0xab),
            _ => None,
        } {
            assert!(
                decoded
                    .iter()
                    .any(|instruction| instruction.opcode == opcode),
                "{name}"
            );
        }
        let ir = ir::decompile_method(&class, method).unwrap();
        let switches: Vec<_> = ir
            .blocks
            .iter_enumerated()
            .filter_map(|(id, block)| {
                if let ir::Terminator::Switch {
                    value,
                    cases,
                    default,
                } = &block.terminator
                {
                    assert!(
                        cases.iter().any(|(_, target)| target == default),
                        "{name}: {cases:?}"
                    );
                    Some((
                        id,
                        *value,
                        cases.clone(),
                        *default,
                        block.terminator.successors(),
                    ))
                } else {
                    None
                }
            })
            .collect();
        assert!(!switches.is_empty(), "{name}");
        let prepared = structure::PreparedIr::new(ir);
        for (id, selector, original_cases, original_default, successors) in switches {
            let block = &prepared.ir().blocks[id];
            let ir::Terminator::Switch {
                value,
                cases,
                default,
            } = &block.terminator
            else {
                panic!("{name}: lost selector evaluation: {block:?}")
            };
            assert_eq!(*value, selector, "{name}");
            assert_eq!(*default, original_default, "{name}");
            assert_eq!(block.terminator.successors(), successors, "{name}");
            assert_eq!(
                cases,
                &original_cases
                    .into_iter()
                    .filter(|(_, target)| *target != *default)
                    .collect::<Vec<_>>(),
                "{name}"
            );
        }
    }
    let expected = crate::common::execute(&original, "SwitchDefaultTargets");
    assert_eq!(expected.iter().filter(|&&byte| byte == b'\n').count(), 112);
    crate::common::decompile(&original, &source);
    let java = source.join("SwitchDefaultTargets.java");
    let rendered = fs::read_to_string(&java).unwrap();
    for (name, kept) in [
        ("dense", vec![0, 1, 5]),
        ("sparse", vec![-10000, 10000]),
        ("allDefault", vec![]),
        ("emptyDefault", vec![]),
        ("cleanup", vec![0, 1, 5]),
    ] {
        let body = crate::common::source::method(&rendered, name);
        let keys: Vec<i32> = body
            .lines()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix("case ")
                    .map(|key| key.strip_suffix(':').unwrap().parse().unwrap())
            })
            .collect();
        for key in &kept {
            assert!(keys.contains(key), "{name}: {body}");
        }
        assert!(keys.iter().all(|key| kept.contains(key)), "{name}: {body}");
        assert_eq!(body.matches("selector(").count(), 1, "{name}: {body}");
    }
    crate::common::compile(&[java], &rebuilt, None);
    assert_eq!(
        crate::common::execute(&rebuilt, "SwitchDefaultTargets"),
        expected
    );
}

#[test]
fn equivalent_terminal_default_arms_absorb_keys_from_distinct_blocks() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    crate::common::compile(
        &["tests/fixtures/java/SwitchDefaultTargets.java".into()],
        &original,
        None,
    );
    let class =
        ClassFile::parse(&fs::read(original.join("SwitchDefaultTargets.class")).unwrap()).unwrap();
    for name in ["mergedReturns", "defaultFirst"] {
        let method = class
            .methods
            .iter()
            .find(|method| method.name == name)
            .unwrap();
        let decoded = sporeflower::bytecode::decode(&method.code.as_ref().unwrap().bytes).unwrap();
        let (cases, default) = decoded
            .iter()
            .find_map(|instruction| {
                if let sporeflower::bytecode::Operand::Switch { cases, default } =
                    &instruction.operand
                {
                    Some((cases, default))
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(cases.len(), 3, "{name}");
        let mut targets: Vec<_> = cases.iter().map(|(_, target)| *target).collect();
        targets.push(*default);
        targets.sort_unstable();
        targets.dedup();
        // These labels survive exact-target filtering: their return bodies
        // occupy distinct bytecode blocks and only merge in the source plan.
        assert_eq!(targets.len(), 4, "{name}: {cases:?}, default={default}");
    }
    let expected = crate::common::execute(&original, "SwitchDefaultTargets");
    crate::common::decompile(&original, &source);
    let java = source.join("SwitchDefaultTargets.java");
    let rendered = fs::read_to_string(&java).unwrap();
    for name in ["mergedReturns", "defaultFirst"] {
        let body = crate::common::source::method(&rendered, name);
        assert_eq!(body.matches("case ").count(), 1, "{name}: {body}");
        assert!(body.contains("case 1:"), "{name}: {body}");
        assert_eq!(body.matches("default:").count(), 1, "{name}: {body}");
        assert_eq!(body.matches("selector(").count(), 1, "{name}: {body}");
    }
    crate::common::compile(&[java], &rebuilt, None);
    assert_eq!(
        crate::common::execute(&rebuilt, "SwitchDefaultTargets"),
        expected
    );
}
