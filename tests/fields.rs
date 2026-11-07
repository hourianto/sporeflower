use crate::common::checked_stdout as checked;
use sporeflower::{
    classfile::{ClassFile, ConstantPool, Field},
    java::{JavaOptions, decompile_class},
    naming::{NamePlan, RenameOptions},
    program::Program,
};
use std::{fs, process::Command};

#[test]
fn final_field_publication_converges_without_moving_rhs_or_observable_reads() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original");
    let rebuilt = dir.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-g:none", "-d"])
            .arg(&original)
            .arg("tests/fixtures/java/FinalJoin.java"),
    );
    let path = original.join("FinalJoin.class");
    let mut bytes = fs::read(&path).unwrap();
    // Change only field access flags: the exact bytecode is legal for JVM finals,
    // although the corresponding sibling Java catch assignments are rejected.
    fn u16_at(bytes: &[u8], at: usize) -> usize {
        u16::from_be_bytes([bytes[at], bytes[at + 1]]) as usize
    }
    let mut at = 10;
    let mut index = 1;
    while index < u16_at(&bytes, 8) {
        let tag = bytes[at];
        at += 1;
        at += match tag {
            1 => 2 + u16_at(&bytes, at),
            3 | 4 | 9 | 10 | 11 | 12 | 17 | 18 => 4,
            5 | 6 => {
                index += 1;
                8
            }
            7 | 8 | 16 | 19 | 20 => 2,
            15 => 3,
            _ => panic!("unexpected constant tag {tag}"),
        };
        index += 1;
    }
    at += 6;
    at += 2 + 2 * u16_at(&bytes, at);
    let fields = u16_at(&bytes, at);
    at += 2;
    for _ in 0..fields {
        let flags = u16_at(&bytes, at) as u16;
        if flags & 8 == 0 {
            bytes[at..at + 2].copy_from_slice(&(flags | 0x10).to_be_bytes());
        }
        let attributes = u16_at(&bytes, at + 6);
        at += 8;
        for _ in 0..attributes {
            let length = u32::from_be_bytes(bytes[at + 2..at + 6].try_into().unwrap()) as usize;
            at += 6 + length;
        }
    }
    fs::write(&path, &bytes).unwrap();
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("FinalJoin"),
    );
    let classes = [ClassFile::parse(&bytes).unwrap()];
    let program = Program::new(&classes, &[]).unwrap();
    let names = NamePlan::build(&program, &RenameOptions::default()).unwrap();
    let output = decompile_class(&program, &names, 0, &JavaOptions::default()).unwrap();
    assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
    let source = rebuilt.join("FinalJoin.java");
    fs::write(&source, &output.source).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .arg(source),
    );
    let actual = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&rebuilt)
            .arg("FinalJoin"),
    );
    assert_eq!(
        String::from_utf8_lossy(&actual),
        String::from_utf8_lossy(&expected)
    );
    let class = ClassFile::parse(&fs::read(rebuilt.join("FinalJoin.class")).unwrap()).unwrap();
    assert_ne!(
        class
            .fields
            .iter()
            .find(|field| field.name == "value")
            .unwrap()
            .access_flags
            & 0x10,
        0
    );
}

#[test]
fn unwritten_jvm_finals_stay_final_without_becoming_compile_time_constants() {
    let class = ClassFile {
        minor_version: 3,
        major_version: 45,
        access_flags: 0x21,
        name: "DefaultFields".into(),
        super_name: Some("java/lang/Object".into()),
        interfaces: vec![],
        constant_pool: ConstantPool::default(),
        fields: [
            ("number", "I"),
            ("flag", "Z"),
            ("character", "C"),
            ("fraction", "D"),
            ("text", "Ljava/lang/String;"),
        ]
        .into_iter()
        .map(|(name, descriptor)| Field {
            access_flags: 0x19,
            name: name.into(),
            descriptor: descriptor.into(),
            constant_value: None,
            synthetic: false,
        })
        .collect(),
        methods: vec![],
        source_file: None,
        inner_classes: vec![],
        warnings: vec![],
    };
    let classes = [class];
    let program = Program::new(&classes, &[]).unwrap();
    let names = NamePlan::build(&program, &RenameOptions::default()).unwrap();
    let output = decompile_class(&program, &names, 0, &JavaOptions::default()).unwrap();
    assert_eq!(output.failed_methods, 0);
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("DefaultFields.java");
    fs::write(&source, output.source).unwrap();
    let driver = dir.path().join("DefaultReader.java");
    fs::write(
        &driver,
        "class DefaultReader { static int read() { return DefaultFields.number; } }",
    )
    .unwrap();
    let result = crate::common::compile_command()
        .args(["-d"])
        .arg(dir.path())
        .arg(source)
        .arg(driver)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let rebuilt =
        ClassFile::parse(&fs::read(dir.path().join("DefaultFields.class")).unwrap()).unwrap();
    for field in &rebuilt.fields {
        assert_ne!(field.access_flags & 0x10, 0);
        assert!(
            field.constant_value.is_none(),
            "{} unexpectedly became a compile-time constant",
            field.name
        );
    }
    let reader =
        ClassFile::parse(&fs::read(dir.path().join("DefaultReader.class")).unwrap()).unwrap();
    let method = reader.methods.iter().find(|m| m.name == "read").unwrap();
    let instructions = sporeflower::bytecode::decode(&method.code.as_ref().unwrap().bytes).unwrap();
    assert!(
        instructions
            .iter()
            .any(|instruction| instruction.opcode == 0xb2),
        "the caller must retain getstatic and its class-initialization boundary"
    );
}

#[test]
fn repeated_constantvalue_stores_do_not_duplicate_final_assignment() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("RepeatedConstant.java");
    fs::write(
        &source,
        "public class RepeatedConstant { public static final int n; static { n = 7; } }",
    )
    .unwrap();
    let compiled = crate::common::compile_command()
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let mut class =
        ClassFile::parse(&fs::read(dir.path().join("RepeatedConstant.class")).unwrap()).unwrap();
    // Old compiler/optimizer output may contain both ConstantValue and the
    // identical putstatic. Model that valid class-file shape without running
    // any corpus application code.
    let constant = class.constant_pool.entries.len() as u16;
    class
        .constant_pool
        .entries
        .push(sporeflower::classfile::Constant::Integer(7));
    class.fields[0].constant_value = Some(constant);
    for interface in [false, true] {
        let mut class = class.clone();
        if interface {
            class.access_flags = 0x0601;
            class.methods.retain(|method| method.name == "<clinit>");
        }
        let classes = [class];
        let program = Program::new(&classes, &[]).unwrap();
        let names = NamePlan::build(&program, &RenameOptions::default()).unwrap();
        let output = decompile_class(&program, &names, 0, &JavaOptions::default()).unwrap();
        assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
        fs::write(&source, output.source).unwrap();
        let compiled = crate::common::compile_command()
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let rebuilt =
            ClassFile::parse(&fs::read(dir.path().join("RepeatedConstant.class")).unwrap())
                .unwrap();
        assert!(rebuilt.fields[0].constant_value.is_some());
        assert_ne!(rebuilt.fields[0].access_flags & 0x10, 0);
    }
}
