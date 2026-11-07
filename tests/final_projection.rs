//! JVM instance ConstantValue metadata, final updates, and write receivers.
use crate::common::checked;
use sporeflower::classfile::{ClassFile, Constant};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn compile(sources: &[PathBuf], target: &Path) {
    fs::create_dir_all(target).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(target)
            .args(sources),
    );
}

fn u16_at(bytes: &[u8], at: usize) -> usize {
    u16::from_be_bytes([bytes[at], bytes[at + 1]]) as usize
}

fn patch_final(path: &Path, add_constant: bool) {
    patch_final_value(path, add_constant.then_some(4));
}

fn patch_final_value(path: &Path, constant_value: Option<i32>) {
    let mut bytes = fs::read(path).unwrap();
    let class = ClassFile::parse(&bytes).unwrap();
    let mut at = 10;
    let mut index = 1;
    while index < u16_at(&bytes, 8) {
        let tag = bytes[at];
        at += 1;
        at += match tag {
            1 => 2 + u16_at(&bytes, at),
            3 | 4 | 9 | 10 | 11 | 12 => 4,
            5 | 6 => {
                index += 1;
                8
            }
            7 | 8 | 16 => 2,
            15 => 3,
            _ => panic!("unexpected constant {tag}"),
        };
        index += 1;
    }
    at += 6;
    at += 2 + 2 * u16_at(&bytes, at);
    let count = u16_at(&bytes, at);
    at += 2;
    for _ in 0..count {
        let start = at;
        let name = class
            .constant_pool
            .utf8(u16_at(&bytes, at + 2) as u16)
            .unwrap();
        let attributes = u16_at(&bytes, at + 6);
        at += 8;
        for _ in 0..attributes {
            at += 6 + u32::from_be_bytes(bytes[at + 2..at + 6].try_into().unwrap()) as usize;
        }
        if name.eq_ignore_ascii_case("value") {
            let flags = u16_at(&bytes, start) as u16;
            bytes[start..start + 2].copy_from_slice(&(flags | 0x10).to_be_bytes());
            if let Some(number) = constant_value {
                let constant = class
                    .constant_pool
                    .entries
                    .iter()
                    .position(|value| matches!(value, Constant::Integer(value) if *value == number))
                    .unwrap() as u16;
                let attribute = class
                    .constant_pool
                    .entries
                    .iter()
                    .position(
                        |value| matches!(value, Constant::Utf8(name) if name == "ConstantValue"),
                    )
                    .unwrap() as u16;
                bytes[start + 6..start + 8]
                    .copy_from_slice(&((attributes + 1) as u16).to_be_bytes());
                let added: Vec<_> = attribute
                    .to_be_bytes()
                    .into_iter()
                    .chain(2u32.to_be_bytes())
                    .chain(constant.to_be_bytes())
                    .collect();
                bytes.splice(at..at, added);
                at += 8;
            }
        }
    }
    bytes[4..8].copy_from_slice(&[0, 3, 0, 45]);
    fs::write(path, bytes).unwrap();
}

#[test]
fn instance_constants_and_final_updates_preserve_vm_defaults_and_write_timing() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from("tests/fixtures/java/FinalProjection.java")],
        &original,
    );
    patch_final(&original.join("FinalMetadata.class"), true);
    patch_final(&original.join("FinalUpdate.class"), false);
    patch_final(&original.join("FinalOther.class"), false);
    let execute = |root: &Path, other: bool| {
        let mut command = Command::new(crate::common::java());
        command
            .args(["-Xverify:all", "-cp"])
            .arg(root)
            .arg("FinalProjection");
        if other {
            command.arg("other");
        }
        checked(&mut command).stdout
    };
    let expected = execute(&original, false);
    assert_eq!(execute(&original, true), b"7:0:0:\n");
    let input = sporeflower::input::Input::read(&original).unwrap();
    let program = sporeflower::program::Program::new(input.classes(), &[]).unwrap();
    let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
    fs::create_dir_all(&source).unwrap();
    let mut sources = Vec::new();
    for id in 0..program.own_count() {
        let rendered =
            sporeflower::java::decompile_class(&program, &names, id, &Default::default()).unwrap();
        assert_eq!(rendered.failed_methods, 0, "{:?}", rendered.diagnostics);
        if program.class(id).name == "FinalOther" {
            assert!(
                !rendered.source.contains("final int value"),
                "{}",
                rendered.source
            );
        }
        if program.class(id).name == "FinalMetadata" {
            assert!(
                rendered.source.contains("final int value;"),
                "{}",
                rendered.source
            );
            assert!(
                !rendered.source.contains("final int value ="),
                "{}",
                rendered.source
            );
        }
        let path = source.join(format!("{}.java", names.class_name(id)));
        fs::write(&path, rendered.source).unwrap();
        sources.push(path);
    }
    compile(&sources, &rebuilt);
    assert_eq!(execute(&rebuilt, true), execute(&original, true));
    assert_eq!(
        String::from_utf8_lossy(&execute(&rebuilt, false)),
        String::from_utf8_lossy(&expected)
    );
    let rebuilt_metadata =
        ClassFile::parse(&fs::read(rebuilt.join("FinalMetadata.class")).unwrap()).unwrap();
    let field = rebuilt_metadata
        .fields
        .iter()
        .find(|field| field.name == "value")
        .unwrap();
    assert_ne!(field.access_flags & 0x10, 0);
    assert!(field.constant_value.is_none());
}

#[test]
fn source_final_policy_preserves_publications_and_expressible_finals() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from("tests/fixtures/java/FinalContexts.java")],
        &original,
    );
    for class in ["FinalMethodWriter", "FinalRepeated", "FinalLoop"] {
        patch_final(&original.join(format!("{class}.class")), false);
    }
    let expected = crate::common::execute(&original, "FinalContexts");
    crate::common::decompile(&original, &source);
    for class in ["FinalMethodWriter", "FinalRepeated", "FinalLoop"] {
        let text = fs::read_to_string(source.join(format!("{class}.java"))).unwrap();
        assert!(!text.contains("final int value"), "{text}");
    }
    for class in ["FinalOnce", "FinalBranch"] {
        let text = fs::read_to_string(source.join(format!("{class}.java"))).unwrap();
        assert!(text.contains("final int value"), "{text}");
    }
    compile(&crate::common::sources(&source), &rebuilt);
    assert_eq!(expected, crate::common::execute(&rebuilt, "FinalContexts"));
}

#[test]
fn final_publication_inside_exception_dispatch_keeps_its_original_timing() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from("tests/fixtures/java/HelperRecovery.java")],
        &original,
    );
    let path = original.join("HelperRecovery.class");
    patch_final(&path, false);
    let mut bytes = fs::read(&path).unwrap();
    let class = ClassFile::parse(&bytes).unwrap();
    let code = class
        .methods
        .iter()
        .find(|method| method.name == "<clinit>")
        .unwrap()
        .code
        .as_ref()
        .unwrap();
    let offset = bytes
        .windows(code.bytes.len())
        .position(|window| window == code.bytes)
        .unwrap();
    assert_eq!(code.exception_table.len(), 1);
    // Cover the handler itself and the final publication. JVM execution still
    // publishes once, but Java's dispatch loop cannot prove definite assignment.
    let end = (code.bytes.len() - 1) as u16;
    let end_offset = offset + code.bytes.len() + 4;
    bytes[end_offset..end_offset + 2].copy_from_slice(&end.to_be_bytes());
    fs::write(&path, &bytes).unwrap();
    let expected = crate::common::execute(&original, "HelperRecoveryDriver");
    let classes = [ClassFile::parse(&bytes).unwrap()];
    let program = sporeflower::program::Program::new(&classes, &[]).unwrap();
    let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
    let rendered =
        sporeflower::java::decompile_class(&program, &names, 0, &Default::default()).unwrap();
    assert_eq!(rendered.failed_methods, 0, "{:?}", rendered.diagnostics);
    assert_eq!(rendered.fallback_methods, 1, "{}", rendered.source);
    assert!(
        !rendered.source.contains("final int VALUE"),
        "{}",
        rendered.source
    );
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("HelperRecovery.java"), &rendered.source).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    fs::copy(
        original.join("HelperRecoveryDriver.class"),
        rebuilt.join("HelperRecoveryDriver.class"),
    )
    .unwrap();
    crate::common::compile(&crate::common::sources(&source), &rebuilt, Some(&rebuilt));
    assert_eq!(
        expected,
        crate::common::execute(&rebuilt, "HelperRecoveryDriver")
    );
}

#[test]
fn relaxed_constantvalue_fields_keep_conditional_narrowing_and_string_freshness() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from("tests/fixtures/java/FinalConstants.java")],
        &original,
    );
    for class in [
        "FinalConstantReset",
        "FinalConstantInit",
        "FinalConstantStable",
        "FinalConstantResetOrder",
    ] {
        patch_final(&original.join(format!("{class}.class")), true);
    }
    for name in [
        "FinalConstantReset",
        "FinalConstantInit",
        "FinalConstantStable",
        "FinalConstantConsumer",
    ] {
        let path = original.join(format!("{name}.class"));
        let mut bytes = fs::read(&path).unwrap();
        let class = ClassFile::parse(&bytes).unwrap();
        let code = class
            .methods
            .iter()
            .find(|method| method.name == "read")
            .unwrap()
            .code
            .as_ref()
            .unwrap();
        let offset = bytes
            .windows(code.bytes.len())
            .position(|window| window == code.bytes)
            .unwrap();
        // IRETURN itself narrows to the declared byte return type. Removing
        // javac's redundant I2B requires the source consumer to supply its cast.
        let narrow = sporeflower::bytecode::decode(&code.bytes)
            .unwrap()
            .into_iter()
            .find(|instruction| instruction.opcode == 0x91)
            .unwrap();
        bytes[offset + narrow.pc as usize] = 0x00;
        fs::write(path, bytes).unwrap();
    }
    let expected = crate::common::execute(&original, "FinalConstants");
    assert_eq!(expected, b"4\n1\n-127\n-127\n5\n4\nfalse\nfalse\n4\n");
    crate::common::decompile(&original, &source);
    for class in ["FinalConstantReset", "FinalConstantInit"] {
        let text = fs::read_to_string(source.join(format!("{class}.java"))).unwrap();
        assert!(!text.contains("final int value"), "{text}");
    }
    let stable = fs::read_to_string(source.join("FinalConstantStable.java")).unwrap();
    assert!(stable.contains("final int value = 4;"), "{stable}");
    compile(&crate::common::sources(&source), &rebuilt);
    assert_eq!(expected, crate::common::execute(&rebuilt, "FinalConstants"));
}

#[test]
fn mutable_nondefault_constantvalue_before_superclass_initialization_is_explicitly_unsupported() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    compile(
        &[PathBuf::from("tests/fixtures/java/FinalPreparation.java")],
        &original,
    );
    patch_final(&original.join("FinalPreparedChild.class"), true);
    assert_eq!(
        crate::common::execute(&original, "FinalPreparation"),
        b"4:9\n"
    );
    let input = sporeflower::input::Input::read(&original).unwrap();
    for ordinary_writer in [false, true] {
        let mut classes = input.classes().to_vec();
        let child = classes
            .iter()
            .position(|class| class.name == "FinalPreparedChild")
            .unwrap();
        if !ordinary_writer {
            classes[child]
                .methods
                .retain(|method| method.name != "reset");
        }
        let program = sporeflower::program::Program::new(&classes, &[]).unwrap();
        let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
        let error =
            sporeflower::java::decompile_class(&program, &names, child, &Default::default())
                .unwrap_err();
        let text = format!("{error:#}");
        assert!(
            text.contains("ConstantValue of mutable field FinalPreparedChild.value"),
            "{text}"
        );
        assert!(text.contains("superclass initialization"), "{text}");
    }
}

#[test]
fn default_constantvalue_does_not_reset_a_superclass_callback_write() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from(
            "tests/fixtures/java/FinalDefaultPreparation.java",
        )],
        &original,
    );
    patch_final_value(&original.join("FinalZeroChild.class"), Some(0));
    patch_final(&original.join("FinalStableChild.class"), true);
    let expected = crate::common::execute(&original, "FinalDefaultPreparation");
    assert_eq!(expected, b"7:7:7\n4:4\n");
    crate::common::decompile(&original, &source);
    let zero = fs::read_to_string(source.join("FinalZeroChild.java")).unwrap();
    assert!(zero.contains("static int value;"), "{zero}");
    let stable = fs::read_to_string(source.join("FinalStableChild.java")).unwrap();
    assert!(stable.contains("static final int value = 4;"), "{stable}");
    compile(&crate::common::sources(&source), &rebuilt);
    assert_eq!(
        expected,
        crate::common::execute(&rebuilt, "FinalDefaultPreparation")
    );
}
