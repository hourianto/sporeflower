//! Source conversions must preserve both arithmetic and Java 1.3 overload choice.
use crate::common::source::method;
use crate::common::{checked, sources as java_sources};
use std::{fs, path::Path, process::Command};

#[test]
fn unknown_hierarchy_conversions_compile_with_stubs_and_run_with_real_subtypes() {
    use sporeflower::{api::ApiCatalog, input::Input, naming::NamePlan, program::Program};

    let work = tempfile::tempdir().unwrap();
    let fixture = work.path().join("fixture");
    fs::create_dir_all(fixture.join("known")).unwrap();
    fs::create_dir_all(fixture.join("vendor")).unwrap();
    fs::write(
        fixture.join("known/Base.java"),
        "package known; public class Base {}",
    )
    .unwrap();
    fs::write(
        fixture.join("vendor/Missing.java"),
        r#"
package vendor;
public class Missing extends known.Base {
    public static int reads;
    public static Missing fetch() { ++reads; return new Missing(); }
}
"#,
    )
    .unwrap();
    fs::write(
        fixture.join("UnknownHierarchy.java"),
        r#"
import known.Base;
import vendor.Missing;
public class UnknownHierarchy {
    static Base implicit() { return Missing.fetch(); }
    static boolean test() { return Missing.fetch() instanceof Base; }
    static Missing down(Base value) { return (Missing) value; }
    static boolean equal(Base value) { return Missing.fetch() == value; }
    static Base[] array(Missing[] value) { return value; }
    static Object object() { return Missing.fetch(); }
    public static void main(String[] args) {
        System.out.println(implicit() == null);
        System.out.println(test());
        System.out.println(down(implicit()) != null);
        try { down(new Base()); } catch (ClassCastException expected) { System.out.println("bad"); }
        System.out.println(equal(null));
        System.out.println(array(null) == null);
        System.out.println(object() == null);
        System.out.println(Missing.reads);
    }
}
"#,
    )
    .unwrap();
    let original = work.path().join("original");
    crate::common::compile(&java_sources(&fixture), &original, None);
    let project = Input::read(&original.join("UnknownHierarchy.class"))
        .unwrap()
        .into_classes();
    let libraries = Input::read(&original.join("known/Base.class"))
        .unwrap()
        .into_classes();
    let api = ApiCatalog::bundled()
        .unwrap()
        .select_with_classpath(&project, &libraries, &Default::default())
        .unwrap();
    assert_eq!(api.inferred_class_count(), 1);
    let mut known = api.classes.clone();
    known.extend(libraries);
    let program = Program::new(&project, &known).unwrap();
    assert!(program.id("vendor/Missing").is_none());
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let rendered =
        sporeflower::java::decompile_class(&program, &names, 0, &Default::default()).unwrap();
    assert_eq!(rendered.failed_methods, 0, "{:?}", rendered.diagnostics);
    for name in ["implicit", "test", "down", "equal", "array"] {
        assert!(
            method(&rendered.source, name).contains("(java.lang.Object)"),
            "{name}: {}",
            rendered.source
        );
    }
    assert!(
        !method(&rendered.source, "object").contains("(java.lang.Object)"),
        "{}",
        rendered.source
    );
    let generated = work.path().join("UnknownHierarchy.java");
    fs::write(&generated, rendered.source).unwrap();
    let stub = work.path().join("api.jar");
    api.write_jar(&stub).unwrap();
    // Only the known superclass is available alongside the inferred Object-based
    // Missing placeholder when javac checks the regenerated source.
    let declared = work.path().join("declared");
    fs::create_dir_all(declared.join("known")).unwrap();
    fs::copy(
        original.join("known/Base.class"),
        declared.join("known/Base.class"),
    )
    .unwrap();
    let classpath = std::env::join_paths([&stub, &declared]).unwrap();
    let expected = crate::common::execute(&original, "UnknownHierarchy");
    assert_eq!(expected, b"false\ntrue\ntrue\nbad\nfalse\ntrue\nfalse\n5\n");
    let execute = |output: &Path| {
        checked(
            Command::new(crate::common::java())
                .arg("-cp")
                .arg(std::env::join_paths([output, &original]).unwrap())
                .arg("UnknownHierarchy"),
        )
        .stdout
    };
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir(&rebuilt).unwrap();
    checked(
        crate::common::compile_command()
            .arg("-classpath")
            .arg(&classpath)
            .arg("-d")
            .arg(&rebuilt)
            .arg(&generated),
    );
    assert_eq!(expected, execute(&rebuilt));
    if let Some(legacy) = crate::common::legacy_javac() {
        let output = work.path().join("legacy");
        fs::create_dir(&output).unwrap();
        checked(
            crate::common::legacy_command(legacy, &stub)
                .arg("-classpath")
                .arg(classpath)
                .arg("-d")
                .arg(&output)
                .arg(generated)
                .arg(fixture.join("known/Base.java")),
        );
        assert_eq!(expected, execute(&output));
    }
}

#[test]
fn legacy_interface_casts_preserve_targets_and_operand_evaluation() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let api = work.path().join("api.jar");
    crate::common::compile(
        &["tests/fixtures/java/LegacyInterfaceCasts.java".into()],
        &original,
        None,
    );
    // The source's Object views emit no checkcast: these are direct JVM casts
    // between interfaces, including inherited and covariant return clashes.
    let class = sporeflower::classfile::ClassFile::parse(
        &fs::read(original.join("LegacyInterfaceCasts.class")).unwrap(),
    )
    .unwrap();
    for name in ["cast", "primitive", "array"] {
        let method = class
            .methods
            .iter()
            .find(|method| method.name == name)
            .unwrap();
        let casts: Vec<_> = sporeflower::bytecode::decode(&method.code.as_ref().unwrap().bytes)
            .unwrap()
            .into_iter()
            .filter(|instruction| instruction.opcode == 0xc0)
            .collect();
        assert_eq!(casts.len(), 1, "{name}: {casts:?}");
    }
    let expected = crate::common::execute(&original, "LegacyInterfaceCasts");
    assert_eq!(expected, b"true\n11\nfalse\ntrue\ntrue\ntrue\n1111\n");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let text = fs::read_to_string(source.join("LegacyInterfaceCasts.java")).unwrap();
    for name in ["cast", "test", "equal", "primitive", "array"] {
        assert!(
            method(&text, name).contains("(java.lang.Object)"),
            "{name}: {text}"
        );
    }
    for name in ["compatible", "overloaded"] {
        assert!(
            !method(&text, name).contains("(java.lang.Object)"),
            "{name}: {text}"
        );
    }
    let sources = java_sources(&source);
    let rebuilt = work.path().join("rebuilt");
    crate::common::compile(&sources, &rebuilt, None);
    assert_eq!(
        expected,
        crate::common::execute(&rebuilt, "LegacyInterfaceCasts")
    );
    let legacy = work.path().join("legacy");
    if crate::common::legacy_compile(&sources, &legacy, &api) {
        assert_eq!(
            expected,
            crate::common::execute(&legacy, "LegacyInterfaceCasts")
        );
    }

    // Authored names can remove an original return-only signature clash. The
    // legality decision must follow the names actually declared in Java.
    let input = sporeflower::input::Input::read(&original)
        .unwrap()
        .into_classes();
    let api_classes = sporeflower::input::Input::read(&api)
        .unwrap()
        .into_classes();
    let program = sporeflower::program::Program::new(&input, &api_classes).unwrap();
    let target = program
        .resolve_method("StringResult", "value", "()Ljava/lang/String;")
        .unwrap();
    let mut overrides = sporeflower::naming::NameOverrides::default();
    overrides.methods.insert(target, "text".into());
    let names = sporeflower::naming::NamePlan::build_with_overrides(
        &program,
        &Default::default(),
        &overrides,
    )
    .unwrap();
    let mapped = work.path().join("mapped");
    for id in 0..program.own_count() {
        let rendered =
            sporeflower::java::decompile_class(&program, &names, id, &Default::default()).unwrap();
        assert_eq!(rendered.failed_methods, 0, "{:?}", rendered.diagnostics);
        if program.class(id).name == "LegacyInterfaceCasts" {
            for name in ["cast", "test", "equal", "array"] {
                assert!(
                    !method(&rendered.source, name).contains("(java.lang.Object)"),
                    "{name}: {}",
                    rendered.source
                );
            }
        }
        let path = mapped.join(format!("{}.java", names.class_name(id)));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, rendered.source).unwrap();
    }
    let mapped_output = work.path().join("mapped-classes");
    if crate::common::legacy_compile(&java_sources(&mapped), &mapped_output, &api) {
        assert_eq!(
            expected,
            crate::common::execute(&mapped_output, "LegacyInterfaceCasts")
        );
    }
}

#[test]
fn casts_are_kept_only_at_required_arithmetic_and_overload_boundaries() {
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
            .arg("tests/fixtures/java/CastContexts.java"),
    );
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("CastContexts"),
    )
    .stdout;
    let api = work.path().join("api.jar");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let text = fs::read_to_string(source.join("CastContexts.java")).unwrap();
    assert!(!method(&text, "noConflict").contains("(Object)"), "{text}");
    assert!(!method(&text, "objectCase").contains("(Object)"), "{text}");
    assert_eq!(
        method(&text, "forcedPair").matches("(Object)").count(),
        1,
        "{text}"
    );
    assert!(
        !method(&text, "nullSpecific").contains("(String)"),
        "{text}"
    );
    assert!(method(&text, "nullGeneral").contains("(Object)"), "{text}");
    assert!(
        method(&text, "nullAmbiguous").contains("(String)"),
        "{text}"
    );
    assert!(!method(&text, "wideningCall").contains("(long)"), "{text}");
    assert!(method(&text, "forcedLong").contains("(long)"), "{text}");
    assert!(method(&text, "forcedDouble").contains("(double)"), "{text}");
    assert!(
        method(&text, "fromConditional").contains("(long)"),
        "{text}"
    );
    assert!(
        method(&text, "objectCharacters").contains("(Object)"),
        "{text}"
    );
    assert!(!method(&text, "stringValue").contains("(Object)"), "{text}");
    assert_eq!(
        method(&text, "product").matches("(long)").count(),
        1,
        "{text}"
    );
    assert!(!method(&text, "literalTimes").contains("(long)"), "{text}");
    assert!(method(&text, "narrowThenWiden").contains("(int)"), "{text}");
    assert!(method(&text, "shift").contains("(long)"), "{text}");
    assert_eq!(
        method(&text, "divide").matches("(double)").count(),
        1,
        "{text}"
    );
    assert!(method(&text, "rounded").contains("(float)"), "{text}");
    assert!(!method(&text, "rounded").contains("(double)"), "{text}");
    let bytes = method(&text, "byteFromZero");
    assert!(
        bytes.contains("byte ") && bytes.contains("++") && bytes.contains("+= 25"),
        "{bytes}"
    );
    assert!(!bytes.contains("= (byte)"), "{bytes}");
    assert!(
        bytes.contains("numeric((int)"),
        "narrow storage must preserve overload selection: {bytes}"
    );
    assert!(method(&text, "shortFromZero").contains("short "), "{text}");
    assert!(
        method(&text, "characterFromZero").contains("char "),
        "{text}"
    );
    assert!(!method(&text, "mixedWrites").contains("byte "), "{text}");
    let small = method(&text, "smallByte");
    assert!(
        small.contains("byte ") && !small.contains("numeric((byte)"),
        "{small}"
    );
    assert!(
        !method(&text, "nullChoice").contains("(String) null"),
        "{text}"
    );
    assert!(
        !method(&text, "nullArrayChoice").contains("(byte[]) null"),
        "{text}"
    );
    let characters = method(&text, "characterSwitch");
    for label in [
        "case ' ':",
        "case '\\'':",
        "case '\\\\':",
        "case '\\n':",
        "case '\\uabcd':",
    ] {
        assert!(characters.contains(label), "{characters}");
    }
    let integers = method(&text, "integerSwitch");
    for label in ["case 32:", "case -1:", "case 65536:"] {
        assert!(integers.contains(label), "{integers}");
    }
    let narrow_switch = method(&text, "narrowSwitch");
    assert!(
        narrow_switch.contains("byte ") && narrow_switch.contains("switch ((int)"),
        "{narrow_switch}"
    );
    let conditional_switch = method(&text, "conditionalNarrowSwitch");
    assert!(
        conditional_switch.contains("switch ((int)"),
        "{conditional_switch}"
    );
    for name in [
        "byteChoice",
        "shortChoice",
        "charChoice",
        "nestedChoice",
        "assignedChoice",
        "choiceConstructor",
        "nestedConstructor",
        "shortConstructor",
    ] {
        let choice = method(&text, name);
        assert!(choice.contains("(int)"), "{name}: {choice}");
    }
    assert!(
        !method(&text, "choiceWithoutOverload").contains("(int)"),
        "{text}"
    );
    assert!(
        !method(&text, "nullStringChoice").contains("(String)"),
        "{text}"
    );
    assert!(
        method(&text, "nullBothChoice").contains("(Object)"),
        "{text}"
    );
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .args(java_sources(&source)),
    );
    assert_eq!(
        expected,
        checked(
            Command::new(crate::common::java())
                .arg("-cp")
                .arg(&rebuilt)
                .arg("CastContexts")
        )
        .stdout
    );
    if let Some(legacy) = crate::common::legacy_javac() {
        let output = work.path().join("legacy");
        fs::create_dir_all(&output).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&output)
                .args(java_sources(&source)),
        );
        assert_eq!(
            expected,
            checked(
                Command::new(crate::common::java())
                    .arg("-cp")
                    .arg(&output)
                    .arg("CastContexts")
            )
            .stdout
        );
    }
}

/// Retain diamonds a Java compiler would normally fold: the separate inner join
/// keeps the String predicate inside the int arm of the outer conditional.
fn constant_choice_bytecode(mut bytes: Vec<u8>) -> Vec<u8> {
    use sporeflower::{
        bytecode::{Operand, decode},
        classfile::{ClassFile, Constant},
    };
    fn u2(bytes: &[u8], at: usize) -> usize {
        u16::from_be_bytes(bytes[at..at + 2].try_into().unwrap()) as usize
    }
    fn u4(bytes: &[u8], at: usize) -> usize {
        u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize
    }
    let class = ClassFile::parse(&bytes).unwrap();
    let string = class
        .constant_pool
        .entries
        .iter()
        .enumerate()
        .find_map(|(index, constant)| {
            matches!(constant, Constant::String(_))
                .then(|| class.constant_pool.string_utf16(index as u16).ok())
                .flatten()
                .filter(|units| units.as_slice() == [b'x' as u16])
                .map(|_| index as u16)
        })
        .unwrap();
    let attribute_name = class
        .constant_pool
        .entries
        .iter()
        .position(|entry| matches!(entry, Constant::Utf8(name) if name == "ConstantValue"))
        .unwrap() as u16;
    let mut at = 10;
    let mut index = 1;
    while index < u2(&bytes, 8) {
        let tag = bytes[at];
        at += 1;
        at += match tag {
            1 => 2 + u2(&bytes, at),
            3 | 4 | 9 | 10 | 11 | 12 => 4,
            5 | 6 => {
                index += 1;
                8
            }
            7 | 8 | 16 => 2,
            15 => 3,
            _ => panic!("unexpected fixture constant {tag}"),
        };
        index += 1;
    }
    at += 6;
    at += 2 + 2 * u2(&bytes, at);
    let fields = u2(&bytes, at);
    at += 2;
    for _ in 0..fields {
        let start = at;
        let name = class.constant_pool.utf8(u2(&bytes, at + 2) as u16).unwrap();
        let attributes = u2(&bytes, at + 6);
        at += 8;
        for _ in 0..attributes {
            at += 6 + u4(&bytes, at + 2);
        }
        if matches!(name, "c" | "d") {
            let flags = u2(&bytes, start) as u16 | 0x10;
            bytes[start..start + 2].copy_from_slice(&flags.to_be_bytes());
            bytes[start + 6..start + 8].copy_from_slice(&((attributes + 1) as u16).to_be_bytes());
            let mut attribute = Vec::new();
            attribute.extend(attribute_name.to_be_bytes());
            attribute.extend(2u32.to_be_bytes());
            attribute.extend(string.to_be_bytes());
            bytes.splice(at..at, attribute);
            at += 8;
        }
    }
    for name in ["literal", "finalFields", "concatenation", "noOverload"] {
        let class = ClassFile::parse(&bytes).unwrap();
        let original = class
            .methods
            .iter()
            .find(|method| method.name == name)
            .unwrap()
            .code
            .as_ref()
            .unwrap();
        let instructions = decode(&original.bytes).unwrap();
        let call = instructions
            .iter()
            .find(|instruction| instruction.opcode == 0xb8)
            .unwrap()
            .pc as usize;
        let outer = instructions
            .iter()
            .find(|instruction| instruction.opcode == 0xa7)
            .unwrap();
        assert!(matches!(outer.operand, Operand::Branch(target) if target as usize == call));
        let mut code = original.bytes.clone();
        for instruction in &instructions {
            if name != "finalFields" && instruction.opcode == 0xb2 {
                let Operand::Constant(index) = instruction.operand else {
                    unreachable!()
                };
                let field = class.constant_pool.member_ref(index).unwrap();
                if matches!(field.name, "a" | "b") {
                    let pc = instruction.pc as usize;
                    code[pc] = 0x13; // ldc_w: keep instruction widths and branch PCs
                    code[pc + 1..pc + 3].copy_from_slice(&string.to_be_bytes());
                }
            }
        }
        let outer = outer.pc as usize;
        code[outer + 1..outer + 3].copy_from_slice(&((call + 3 - outer) as i16).to_be_bytes());
        code.splice(call..call, [0xa7, 0, 3]);
        let positions: Vec<_> = bytes
            .windows(original.bytes.len())
            .enumerate()
            .filter_map(|(at, window)| (window == original.bytes).then_some(at))
            .collect();
        assert_eq!(positions.len(), 1);
        let start = positions[0] - 8;
        let length = u4(&bytes, start - 4);
        let mut attribute = Vec::new();
        attribute.extend(original.max_stack.to_be_bytes());
        attribute.extend(original.max_locals.to_be_bytes());
        attribute.extend((code.len() as u32).to_be_bytes());
        attribute.extend(code);
        attribute.extend([0, 0, 0, 0]); // no exception handlers or stale frame/debug attributes
        bytes[start - 4..start].copy_from_slice(&(attribute.len() as u32).to_be_bytes());
        bytes.splice(start..start + length, attribute);
    }
    bytes[4..8].copy_from_slice(&[0, 0, 0, 49]);
    bytes
}

/// The JVM field has no ConstantValue attribute, but reconstructing its raw
/// interface initializer produces a Java constant variable.
fn interface_constant_bytecode(mut bytes: Vec<u8>) -> Vec<u8> {
    use sporeflower::{
        bytecode::{Operand, decode},
        classfile::{ClassFile, Constant},
    };
    let class = ClassFile::parse(&bytes).unwrap();
    assert!(
        class
            .fields
            .iter()
            .find(|field| field.name == "value")
            .unwrap()
            .constant_value
            .is_none()
    );
    let string = class
        .constant_pool
        .entries
        .iter()
        .enumerate()
        .find_map(|(index, entry)| {
            matches!(entry, Constant::String(_))
                .then(|| class.constant_pool.string_utf16(index as u16).ok())
                .flatten()
                .filter(|value| value.as_slice() == [b'x' as u16])
                .map(|_| index as u16)
        })
        .unwrap();
    let original = &class
        .methods
        .iter()
        .find(|method| method.name == "<clinit>")
        .unwrap()
        .code
        .as_ref()
        .unwrap()
        .bytes;
    let mut code = original.clone();
    let mut replacements = 0;
    for instruction in decode(original).unwrap() {
        if instruction.opcode == 0xb2 {
            let Operand::Constant(index) = instruction.operand else {
                unreachable!()
            };
            let field = class.constant_pool.member_ref(index).unwrap();
            assert!(matches!(field.name, "a" | "b"));
            let pc = instruction.pc as usize;
            code[pc] = 0x13;
            code[pc + 1..pc + 3].copy_from_slice(&string.to_be_bytes());
            replacements += 1;
        }
    }
    assert_eq!(replacements, 2);
    let positions: Vec<_> = bytes
        .windows(original.len())
        .enumerate()
        .filter_map(|(at, window)| (window == original).then_some(at))
        .collect();
    assert_eq!(positions.len(), 1);
    let start = positions[0];
    bytes[start..start + code.len()].copy_from_slice(&code);
    bytes[4..8].copy_from_slice(&[0, 0, 0, 49]);
    bytes
}

#[test]
fn uncertain_source_constants_preserve_overloads_without_casting_every_choice() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let modern = work.path().join("modern");
    let legacy_output = work.path().join("legacy");
    let api = work.path().join("api.jar");
    for path in [&original, &modern, &legacy_output] {
        fs::create_dir_all(path).unwrap();
    }
    checked(
        crate::common::compile_command()
            .args(["-g:none", "-d"])
            .arg(&original)
            .arg("tests/fixtures/java/ConstantChoiceContexts.java"),
    );
    let class = original.join("ConstantChoiceContexts.class");
    fs::write(&class, constant_choice_bytecode(fs::read(&class).unwrap())).unwrap();
    let interface = original.join("RawConstantChoice.class");
    fs::write(
        &interface,
        interface_constant_bytecode(fs::read(&interface).unwrap()),
    )
    .unwrap();
    let execute = |classes: &Path| {
        checked(
            Command::new(crate::common::java())
                .args(["-Xverify:all", "-cp"])
                .arg(classes)
                .arg("ConstantChoiceContexts"),
        )
        .stdout
    };
    let expected = execute(&original);
    assert_eq!(
        String::from_utf8_lossy(&expected),
        "20\n20\n20\n30\n20\n20\n20\n20\n20\n30\n20\n20\n"
    );
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let text = fs::read_to_string(source.join("ConstantChoiceContexts.java")).unwrap();
    for name in ["literal", "finalFields", "concatenation", "interfaceField"] {
        assert!(method(&text, name).contains("pick((int)"), "{text}");
    }
    assert!(!method(&text, "noOverload").contains("(int)"), "{text}");
    assert!(!method(&text, "mutableField").contains("(int)"), "{text}");
    let interface = fs::read_to_string(source.join("RawConstantChoice.java")).unwrap();
    assert!(interface.contains("int value = \"x\""), "{interface}");
    assert!(
        method(&text, "concatenation").contains("new String("),
        "constant concatenation must retain fresh identity: {text}"
    );
    let sources = java_sources(&source);
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&modern)
            .args(&sources),
    );
    assert_eq!(execute(&modern), expected, "{text}");
    if let Some(legacy) = crate::common::legacy_javac() {
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&legacy_output)
                .args(&sources),
        );
        assert_eq!(execute(&legacy_output), expected, "{text}");
    }
}
