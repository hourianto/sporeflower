use crate::common::{checked, compile, files};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[test]
fn final_assignment_after_this_delegation_keeps_argument_super_and_branch_effects() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from(
            "tests/fixtures/java/ConstructorFinalDelegation.java",
        )],
        &original,
        None,
    );
    let path = original.join("ConstructorFinalDelegation.class");
    let mut bytes = fs::read(&path).unwrap();
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
            3 | 4 | 9 | 10 | 11 | 12 => 4,
            5 | 6 => {
                index += 1;
                8
            }
            7 | 8 | 16 => 2,
            15 => 3,
            _ => panic!("unexpected tag {tag}"),
        };
        index += 1;
    }
    at += 6;
    at += 2 + 2 * u16_at(&bytes, at);
    assert_eq!(u16_at(&bytes, at), 3);
    at += 2;
    for _ in 0..3 {
        let flags = u16_at(&bytes, at) as u16;
        bytes[at..at + 2].copy_from_slice(&(flags | 0x10).to_be_bytes());
        assert_eq!(u16_at(&bytes, at + 6), 0);
        at += 8;
    }
    fs::write(&path, bytes).unwrap();
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("ConstructorFinalDelegation"),
    )
    .stdout;
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .args(["--no-api", "--threads", "1"]),
    );
    let emitted = fs::read_to_string(source.join("ConstructorFinalDelegation.java")).unwrap();
    assert!(emitted.contains("final int value;"), "{emitted}");
    assert!(emitted.contains("final int other;"), "{emitted}");
    assert!(
        emitted.contains("++]"),
        "delegated postfix result must retain its update: {emitted}"
    );
    assert!(
        emitted.contains("$preinit"),
        "effectful unused arguments must remain before super: {emitted}"
    );
    compile(&files(&source, "java"), &rebuilt, None);
    let actual = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&rebuilt)
            .arg("ConstructorFinalDelegation"),
    )
    .stdout;
    assert_eq!(
        String::from_utf8_lossy(&actual),
        String::from_utf8_lossy(&expected),
        "{emitted}"
    );
    let class = sporeflower::classfile::ClassFile::parse(
        &fs::read(rebuilt.join("ConstructorFinalDelegation.class")).unwrap(),
    )
    .unwrap();
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
fn branching_and_effectful_constructor_preludes_roundtrip() {
    let root = Path::new("tests/fixtures/constructors");
    let work = tempfile::tempdir().unwrap();
    let support = work.path().join("support");
    let original = work.path().join("original");
    let output = work.path().join("source");
    let recompiled = work.path().join("recompiled");
    let support_sources = files(&root.join("support"), "java");
    compile(&support_sources, &support, None);
    fs::create_dir_all(original.join("pkg")).unwrap();
    for class in files(&root.join("classes"), "class") {
        fs::copy(
            &class,
            original.join("pkg").join(class.file_name().unwrap()),
        )
        .unwrap();
    }
    for class in files(&support, "class") {
        fs::copy(&class, original.join(class.strip_prefix(&support).unwrap())).unwrap();
    }
    let driver = root.join("ConstructorDriver.java");
    compile(std::slice::from_ref(&driver), &original, Some(&original));
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("ConstructorDriver"),
    )
    .stdout;
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(root.join("classes"))
            .arg(&output)
            .args(["--no-api", "--threads", "1", "--classpath"])
            .arg(&support),
    );
    let mut source = files(&output, "java");
    source.extend(support_sources);
    source.push(driver);
    compile(&source, &recompiled, None);
    let actual = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&recompiled)
            .arg("ConstructorDriver"),
    )
    .stdout;
    assert_eq!(
        String::from_utf8_lossy(&actual),
        String::from_utf8_lossy(&expected)
    );
    let saved =
        fs::read_to_string(output.join("pkg/TestConstructorSavedArgumentsBeforeSuper.java"))
            .unwrap();
    assert!(
        !saved.contains("$Preinit"),
        "pure saved arguments do not require a carrier:\n{saved}"
    );
}

fn render_class(class: sporeflower::classfile::ClassFile) -> sporeflower::java::DecompiledClass {
    let classes = [class];
    let program = sporeflower::program::Program::new(&classes, &[]).unwrap();
    let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
    sporeflower::java::decompile_class(&program, &names, 0, &Default::default()).unwrap()
}

#[test]
fn local_values_cross_super_without_reordering_argument_effects() {
    let root = Path::new("tests/fixtures/constructors");
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let recompiled = work.path().join("recompiled");
    let support = root.join("support/pkg/TestConstructorPreludeEffects.java");
    compile(std::slice::from_ref(&support), &original, None);
    let bytes = fs::read(root.join("boundary/TestConstructorUnliftablePrelude.class")).unwrap();
    fs::write(
        original.join("pkg/TestConstructorUnliftablePrelude.class"),
        &bytes,
    )
    .unwrap();
    let driver = root.join("ConstructorLiveDriver.java");
    compile(std::slice::from_ref(&driver), &original, Some(&original));
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("ConstructorLiveDriver"),
    )
    .stdout;
    let mut class = sporeflower::classfile::ClassFile::parse(&bytes).unwrap();
    // The zero-argument member writes uninitialized this, an independently
    // asserted unsupported boundary below. Isolate the two expressible members.
    class
        .methods
        .retain(|method| method.name != "<init>" || method.descriptor != "()V");
    let rendered = render_class(class);
    assert_eq!(rendered.failed_methods, 0, "{:?}", rendered.diagnostics);
    let source = work.path().join("TestConstructorUnliftablePrelude.java");
    fs::write(&source, &rendered.source).unwrap();
    compile(&[source, support, driver], &recompiled, None);
    let actual = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&recompiled)
            .arg("ConstructorLiveDriver"),
    )
    .stdout;
    assert_eq!(
        String::from_utf8_lossy(&actual),
        String::from_utf8_lossy(&expected)
    );
}

#[test]
fn uninitialized_receiver_write_is_not_silently_moved_after_super() {
    let bytes =
        fs::read("tests/fixtures/constructors/boundary/TestConstructorUnliftablePrelude.class")
            .unwrap();
    let rendered = render_class(sporeflower::classfile::ClassFile::parse(&bytes).unwrap());
    assert_eq!(rendered.failed_methods, 1, "{:?}", rendered.diagnostics);
    assert!(
        rendered
            .diagnostics
            .iter()
            .any(|message| message.contains("uninitialized this"))
    );
}

#[test]
fn ordinary_super_delegation_precedes_local_declarations() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let output = work.path().join("source");
    let recompiled = work.path().join("recompiled");
    compile(
        &[PathBuf::from(
            "tests/fixtures/java/ConstructorStraight.java",
        )],
        &original,
        None,
    );
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("ConstructorStraight"),
    )
    .stdout;
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&output)
            .args(["--no-api", "--threads", "1"]),
    );
    let source = fs::read_to_string(output.join("ConstructorStraight.java")).unwrap();
    assert!(
        !source.contains("$Preinit"),
        "ordinary constructors should remain ordinary:\n{source}"
    );
    compile(&files(&output, "java"), &recompiled, None);
    let actual = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&recompiled)
            .arg("ConstructorStraight"),
    )
    .stdout;
    assert_eq!(actual, expected);
}

#[test]
fn post_delegation_catch_local_is_defined_by_its_handler() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let output = work.path().join("source");
    let recompiled = work.path().join("recompiled");
    compile(
        &[PathBuf::from("tests/fixtures/java/ConstructorCatch.java")],
        &original,
        None,
    );
    let path = original.join("ConstructorCatch.class");
    let mut bytes = fs::read(&path).unwrap();
    let class = sporeflower::classfile::ClassFile::parse(&bytes).unwrap();
    let method = class
        .methods
        .iter()
        .find(|method| method.name == "<init>")
        .unwrap();
    let code = &method.code.as_ref().unwrap().bytes;
    let instructions = sporeflower::bytecode::decode(code).unwrap();
    let delegation = instructions
        .iter()
        .position(|instruction| instruction.opcode == 0xb7)
        .unwrap();
    let before = instructions
        .iter()
        .position(|instruction| instruction.opcode == 0xb8)
        .unwrap();
    let start = instructions[delegation + 1].pc as usize;
    let end = instructions[before + 1].pc as usize;
    let reordered: Vec<_> = code[start..end]
        .iter()
        .chain(&code[..start])
        .copied()
        .collect();
    let offset = bytes
        .windows(code.len())
        .position(|window| window == code)
        .unwrap();
    bytes[offset..offset + end].copy_from_slice(&reordered);
    fs::write(path, bytes).unwrap();
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("ConstructorCatch"),
    )
    .stdout;
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&output)
            .args(["--no-api", "--threads", "1"]),
    );
    let source = fs::read_to_string(output.join("ConstructorCatch.java")).unwrap();
    assert!(
        source.contains("$Preinit"),
        "fixture must exercise constructor region extraction: {source}"
    );
    compile(&files(&output, "java"), &recompiled, None);
    let actual = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&recompiled)
            .arg("ConstructorCatch"),
    )
    .stdout;
    assert_eq!(actual, expected);
}

#[test]
fn bounded_constructor_arguments_fall_back_to_a_carrier_with_valid_delegation() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from("tests/fixtures/java/ConstructorDepth.java")],
        &original,
        None,
    );
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .args(["--no-api", "--threads", "1"]),
    );
    let emitted = fs::read_to_string(source.join("ConstructorDepth.java")).unwrap();
    assert!(emitted.contains("$preinit"), "{emitted}");
    assert!(
        emitted.contains("class $Preinit0_"),
        "referenced source type must not be shadowed: {emitted}"
    );
    compile(&files(&source, "java"), &rebuilt, None);
    assert_eq!(
        crate::common::execute(&original, "ConstructorDepth"),
        b"607\n29\n"
    );
    assert_eq!(
        crate::common::execute(&rebuilt, "ConstructorDepth"),
        b"607\n29\n"
    );
}

#[test]
fn unsupported_constructor_bodies_keep_accessible_delegation_without_new_checked_throws() {
    use sporeflower::{classfile::ClassFile, naming::NamePlan, program::Program};
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("FailureChild.java");
    fs::write(
        &source,
        r#"
class FailureBase {
    private FailureBase() {}
    FailureBase(String value) throws java.io.IOException {}
    FailureBase(int value) {}
    FailureBase(char value) {}
}
public class FailureChild extends FailureBase {
    public FailureChild() { super('x'); }
    public FailureChild(boolean ignored) { super(7); }
}
"#,
    )
    .unwrap();
    compile(std::slice::from_ref(&source), &original, None);
    let mut child =
        ClassFile::parse(&fs::read(original.join("FailureChild.class")).unwrap()).unwrap();
    let base = ClassFile::parse(&fs::read(original.join("FailureBase.class")).unwrap()).unwrap();
    for method in &mut child.methods {
        let code = method.code.as_mut().unwrap();
        if method.descriptor == "()V" {
            // Keep its exact char delegation recoverable, but leave an acquired
            // monitor at return so ordinary source reconstruction must fail.
            assert_eq!(code.bytes.pop(), Some(0xb1));
            code.bytes.extend([0x2a, 0xc2, 0xb1]);
        } else {
            // Lifting cannot recover this delegation. The fallback must skip
            // private super() and the String overload's undeclared IOException.
            code.bytes = vec![0xca];
        }
    }
    let classes = [child];
    let libraries = [base];
    let program = Program::new(&classes, &libraries).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let rendered =
        sporeflower::java::decompile_class(&program, &names, 0, &Default::default()).unwrap();
    assert_eq!(rendered.failed_methods, 2, "{:?}", rendered.diagnostics);
    assert!(
        rendered.source.contains("super((char) 0);"),
        "{}",
        rendered.source
    );
    assert!(rendered.source.contains("super(0);"), "{}", rendered.source);
    assert!(!rendered.source.contains("throws "), "{}", rendered.source);
    fs::write(&source, &rendered.source).unwrap();
    compile(
        std::slice::from_ref(&source),
        &work.path().join("rebuilt"),
        Some(&original),
    );

    // The lifted invocation still provides its exact descriptor when the
    // superclass itself was not supplied to the decompiler.
    let mut child = classes[0].clone();
    child.methods.retain(|method| method.descriptor == "()V");
    let classes = [child];
    let program = Program::new(&classes, &[]).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let rendered =
        sporeflower::java::decompile_class(&program, &names, 0, &Default::default()).unwrap();
    assert!(
        rendered.source.contains("super((char) 0);"),
        "{}",
        rendered.source
    );
    fs::write(&source, &rendered.source).unwrap();
    compile(
        &[source],
        &work.path().join("without-parent-metadata"),
        Some(&original),
    );
}
