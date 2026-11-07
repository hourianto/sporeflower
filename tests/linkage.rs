//! Exact descriptor failures retain operand ordering and target initialization.
use crate::common::checked;
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

fn replace(bytes: &mut [u8], old: &[u8], new: &[u8]) {
    assert_eq!(old.len(), new.len());
    let positions: Vec<_> = bytes
        .windows(old.len())
        .enumerate()
        .filter_map(|(index, value)| (value == old).then_some(index))
        .collect();
    assert_eq!(positions.len(), 1);
    bytes[positions[0]..positions[0] + old.len()].copy_from_slice(new);
}

#[test]
fn unresolved_exact_members_preserve_resolution_and_operand_order() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from("tests/fixtures/java/LinkageProbe.java")],
        &original,
    );
    let caller = original.join("LinkageCaller.class");
    let mut bytes = fs::read(&caller).unwrap();
    replace(&mut bytes, b"oldStatic", b"badStatic");
    replace(&mut bytes, b"oldVirtual", b"badVirtual");
    replace(&mut bytes, b"oldSpecial", b"badSpecial");
    replace(&mut bytes, b"(LLinkageNarrow;I)V", b"(LLinkageBroad_;I)V");
    fs::write(caller, bytes).unwrap();
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .args(["--no-api", "--threads", "1"]),
    );
    let emitted = fs::read_to_string(source.join("LinkageCaller.java")).unwrap();
    assert!(
        emitted.contains("new java.lang.NoSuchMethodError("),
        "{emitted}"
    );
    let sources: Vec<_> = fs::read_dir(&source)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "java")
        })
        .collect();
    compile(&sources, &rebuilt);
    for method in ["static", "virtual", "special", "new", "reflect"] {
        for mode in 0..3 {
            let execute = |root: &Path| {
                checked(
                    Command::new(crate::common::java())
                        .arg("-cp")
                        .arg(root)
                        .arg("LinkageProbe")
                        .arg(method)
                        .arg(mode.to_string()),
                )
                .stdout
            };
            let expected = execute(&original);
            let actual = execute(&rebuilt);
            assert_eq!(
                String::from_utf8_lossy(&actual),
                String::from_utf8_lossy(&expected),
                "{method}, mode {mode}\n{emitted}"
            );
        }
    }

    // CLDC omits NoSuchMethodError; expose the supported nearest Error, without
    // introducing a fabricated java.lang class or changing the target methods.
    let input = sporeflower::input::Input::read(&original).unwrap();
    let api = sporeflower::api::ApiCatalog::bundled()
        .unwrap()
        .select(input.classes(), &Default::default())
        .unwrap();
    let program = sporeflower::program::Program::new(input.classes(), &api.classes).unwrap();
    let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
    let id = program.id("LinkageCaller").unwrap();
    let rendered =
        sporeflower::java::decompile_class(&program, &names, id, &Default::default()).unwrap();
    assert_eq!(rendered.failed_methods, 0, "{:?}", rendered.diagnostics);
    assert!(
        rendered.source.contains("new java.lang.Error("),
        "{}",
        rendered.source
    );
    assert!(
        rendered
            .diagnostics
            .iter()
            .any(|message| message.contains("NoSuchMethodError"))
    );
    let cldc_source = work.path().join("cldc-source");
    let cldc_classes = work.path().join("cldc-classes");
    fs::create_dir_all(&cldc_source).unwrap();
    let mut sources = Vec::new();
    for id in 0..program.own_count() {
        let rendered =
            sporeflower::java::decompile_class(&program, &names, id, &Default::default()).unwrap();
        assert_eq!(rendered.failed_methods, 0, "{:?}", rendered.diagnostics);
        let path = cldc_source.join(format!("{}.java", names.class_name(id)));
        fs::write(&path, rendered.source).unwrap();
        sources.push(path);
    }
    compile(&sources, &cldc_classes);
    for (method, mode) in [
        ("static", 1),
        ("virtual", 0),
        ("new", 0),
        ("new", 1),
        ("new", 2),
        ("reflect", 0),
    ] {
        let execute = |root: &Path| {
            checked(
                Command::new(crate::common::java())
                    .arg("-cp")
                    .arg(root)
                    .arg("LinkageProbe")
                    .arg(method)
                    .arg(mode.to_string()),
            )
            .stdout
        };
        let expected = String::from_utf8(execute(&original))
            .unwrap()
            .replace("java.lang.NoSuchMethodError", "java.lang.Error");
        assert_eq!(
            String::from_utf8(execute(&cldc_classes)).unwrap(),
            expected,
            "CLDC {method}, mode {mode}"
        );
    }
}

#[test]
fn generated_linkage_contracts_prepare_interface_initializers_and_checked_catches() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from("tests/fixtures/java/JavaPhaseProbe.java")],
        &original,
    );
    for owner in ["PhaseInterface", "PhaseCaller"] {
        let path = original.join(format!("{owner}.class"));
        let mut bytes = fs::read(&path).unwrap();
        replace(&mut bytes, b"present", b"missing");
        fs::write(path, bytes).unwrap();
    }
    let api = work.path().join("api.jar");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let interface = fs::read_to_string(source.join("PhaseInterface.java")).unwrap();
    let caller = fs::read_to_string(source.join("PhaseCaller.java")).unwrap();
    assert!(interface.contains("$Init.$sf$linkage"), "{interface}");
    assert!(
        caller.contains("if (false)") && caller.contains("IOException"),
        "{caller}"
    );
    let sources = crate::common::files(&source, "java");
    compile(&sources, &rebuilt);
    let legacy = work.path().join("legacy");
    let has_legacy = crate::common::legacy_compile(&sources, &legacy, &api);
    for mode in ["interface", "method"] {
        let execute = |classes: &Path| {
            String::from_utf8(
                checked(
                    Command::new(crate::common::java())
                        .args(["-Xverify:all", "-cp"])
                        .arg(classes)
                        .arg("JavaPhaseProbe")
                        .arg(mode),
                )
                .stdout,
            )
            .unwrap()
            .replace("java.lang.NoSuchMethodError", "java.lang.Error")
        };
        let expected = execute(&original);
        assert_eq!(
            expected,
            if mode == "interface" {
                "java.lang.Error\n12\n"
            } else {
                "java.lang.Error\n4\n"
            }
        );
        assert_eq!(execute(&rebuilt), expected, "{interface}\n{caller}");
        if has_legacy {
            assert_eq!(execute(&legacy), expected);
        }
    }
}

#[test]
fn interface_helper_dispatch_is_counted_on_its_original_initializer() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from("tests/fixtures/java/HelperRecovery.java")],
        &original,
    );
    let mut class = sporeflower::classfile::ClassFile::parse(
        &fs::read(original.join("HelperRecovery.class")).unwrap(),
    )
    .unwrap();
    class.access_flags = 0x0601;
    class.methods.retain(|method| method.name == "<clinit>");
    for field in &mut class.fields {
        field.access_flags = 0x0019;
    }
    let code = class.methods[0].code.as_mut().unwrap();
    let instructions = sporeflower::bytecode::decode(&code.bytes).unwrap();
    // The handler's invocation is covered by that same handler. This legal JVM
    // exception cycle has no Java catch nesting representation and needs dispatch.
    let end = instructions
        .iter()
        .find(|instruction| instruction.opcode == 0xb3)
        .unwrap()
        .pc;
    code.exception_table[0].end_pc = end;
    let classes = [class];
    let program = sporeflower::program::Program::new(&classes, &[]).unwrap();
    let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
    let rendered =
        sporeflower::java::decompile_class(&program, &names, 0, &Default::default()).unwrap();
    assert_eq!(
        rendered.failed_methods, 0,
        "{:?}\n{}",
        rendered.diagnostics, rendered.source
    );
    assert_eq!(rendered.fallback_methods, 1, "{}", rendered.source);
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("HelperRecovery.java"), &rendered.source).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    fs::copy(
        original.join("HelperRecoveryDriver.class"),
        rebuilt.join("HelperRecoveryDriver.class"),
    )
    .unwrap();
    crate::common::compile(
        &[source.join("HelperRecovery.java")],
        &rebuilt,
        Some(&rebuilt),
    );
    assert_eq!(
        crate::common::execute(&original, "HelperRecoveryDriver"),
        b"42\n"
    );
    assert_eq!(
        crate::common::execute(&rebuilt, "HelperRecoveryDriver"),
        b"42\n"
    );
}

#[test]
fn missing_fields_and_inaccessible_members_preserve_operands_and_failures() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    let target = work.path().join("Target.java");
    fs::write(
        &target,
        r#"
package linkagebase;
public class Target {
    static { System.out.println("target-init"); }
    public int oldField;
    public static int oldShared;
    public int available(int value) { return value; }
    protected int forbidden(int value) { return value; }
}
"#,
    )
    .unwrap();
    let probe = work.path().join("MemberLinkageProbe.java");
    fs::write(
        &probe,
        r#"
class MemberLinkageState { static String trace = ""; }
class MemberLinkageCtor {
    static { MemberLinkageState.trace += "init;"; }
    public MemberLinkageCtor(boolean value) {}
    private MemberLinkageCtor(byte value) {}
}
class MemberLinkageCaller {
    static linkagebase.Target receiver() { MemberLinkageState.trace += "receiver;"; return null; }
    static int argument() { MemberLinkageState.trace += "argument;"; return 4; }
    static boolean flag() { MemberLinkageState.trace += "flag;"; return false; }
    static int read() { return receiver().oldField; }
    static void write() { receiver().oldField = argument(); }
    static int compound() { return receiver().oldField += argument(); }
    static int update() { return receiver().oldField++; }
    static int staticRead() { return linkagebase.Target.oldShared; }
    static void staticWrite() { linkagebase.Target.oldShared = argument(); }
    static int access() { return receiver().available(argument()); }
    static Object construct() { return new MemberLinkageCtor(flag()); }
}
public class MemberLinkageProbe {
    public static void main(String[] args) {
        try {
            if (args[0].equals("read")) MemberLinkageCaller.read();
            else if (args[0].equals("write")) MemberLinkageCaller.write();
            else if (args[0].equals("compound")) MemberLinkageCaller.compound();
            else if (args[0].equals("update")) MemberLinkageCaller.update();
            else if (args[0].equals("staticRead")) MemberLinkageCaller.staticRead();
            else if (args[0].equals("staticWrite")) MemberLinkageCaller.staticWrite();
            else if (args[0].equals("access")) MemberLinkageCaller.access();
            else MemberLinkageCaller.construct();
        } catch (Throwable failure) { System.out.println(failure.getClass().getName()); }
        System.out.println(MemberLinkageState.trace);
    }
}
"#,
    )
    .unwrap();
    compile(&[target, probe], &original);
    let caller = original.join("MemberLinkageCaller.class");
    let mut bytes = fs::read(&caller).unwrap();
    for (old, new) in [
        (b"oldField".as_slice(), b"badField".as_slice()),
        (b"oldShared", b"badShared"),
        (b"available", b"forbidden"),
        (b"(Z)V", b"(B)V"),
    ] {
        replace(&mut bytes, old, new);
    }
    fs::write(caller, bytes).unwrap();
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .args(["--no-api", "--threads", "1"]),
    );
    let emitted = fs::read_to_string(source.join("MemberLinkageCaller.java")).unwrap();
    assert!(emitted.contains("NoSuchFieldError"), "{emitted}");
    assert!(emitted.contains("IllegalAccessError"), "{emitted}");
    compile(&crate::common::sources(&source), &rebuilt);
    for mode in [
        "read",
        "write",
        "compound",
        "update",
        "staticRead",
        "staticWrite",
        "access",
        "construct",
    ] {
        let execute = |root: &Path| {
            checked(
                Command::new(crate::common::java())
                    .arg("-cp")
                    .arg(root)
                    .arg("MemberLinkageProbe")
                    .arg(mode),
            )
            .stdout
        };
        assert_eq!(execute(&rebuilt), execute(&original), "{mode}\n{emitted}");
    }
}

#[test]
fn impossible_original_hierarchies_have_explicit_class_diagnostics() {
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("HierarchyProbe.java");
    fs::write(&source, "class HierarchyBase { public void method() {} } class HierarchyProbe extends HierarchyBase { public void method() {} }").unwrap();
    compile(&[source], work.path());
    let input = sporeflower::input::Input::read(work.path()).unwrap();
    for final_class in [false, true] {
        let mut classes = input.classes().to_vec();
        let parent = classes
            .iter_mut()
            .find(|class| class.name == "HierarchyBase")
            .unwrap();
        if final_class {
            parent.access_flags |= 0x10;
        } else {
            parent
                .methods
                .iter_mut()
                .find(|method| method.name == "method")
                .unwrap()
                .access_flags |= 0x10;
        }
        let program = sporeflower::program::Program::new(&classes, &[]).unwrap();
        let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
        let failure = sporeflower::java::decompile_class(
            &program,
            &names,
            program.id("HierarchyProbe").unwrap(),
            &Default::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            failure.contains(if final_class {
                "extends final superclass"
            } else {
                "overrides final method"
            }),
            "{failure}"
        );
    }
}

#[test]
fn forbidden_final_writes_preserve_reads_operands_initialization_and_error_order() {
    fn patch_fields(path: &Path, major: u16, constant_value: bool) {
        let mut bytes = fs::read(path).unwrap();
        let u16_at =
            |bytes: &[u8], at: usize| u16::from_be_bytes([bytes[at], bytes[at + 1]]) as usize;
        let mut at = 10;
        let mut index = 1;
        let mut value_name = None;
        let mut attribute_name = None;
        let mut constant = None;
        while index < u16_at(&bytes, 8) {
            let tag = bytes[at];
            at += 1;
            if tag == 1 {
                let name = &bytes[at + 2..at + 2 + u16_at(&bytes, at)];
                if name == b"value" {
                    value_name = Some(index);
                }
                if name == b"ConstantValue" {
                    attribute_name = Some(index);
                }
            } else if tag == 3 && bytes[at..at + 4] == 4i32.to_be_bytes() {
                constant = Some(index);
            }
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
            let flags = u16_at(&bytes, at) as u16;
            bytes[at..at + 2].copy_from_slice(&(flags | 0x10).to_be_bytes());
            let attributes = u16_at(&bytes, at + 6);
            let add_constant = constant_value && Some(u16_at(&bytes, at + 2)) == value_name;
            if add_constant {
                bytes[at + 6..at + 8].copy_from_slice(&((attributes + 1) as u16).to_be_bytes());
            }
            at += 8;
            for _ in 0..attributes {
                at += 6 + u32::from_be_bytes(bytes[at + 2..at + 6].try_into().unwrap()) as usize;
            }
            if add_constant {
                let mut attribute = Vec::new();
                attribute.extend_from_slice(&(attribute_name.unwrap() as u16).to_be_bytes());
                attribute.extend_from_slice(&2u32.to_be_bytes());
                attribute.extend_from_slice(&(constant.unwrap() as u16).to_be_bytes());
                bytes.splice(at..at, attribute);
                at += 8;
            }
        }
        bytes[4..6].copy_from_slice(&(if major == 45 { 3u16 } else { 0u16 }).to_be_bytes());
        bytes[6..8].copy_from_slice(&major.to_be_bytes());
        fs::write(path, bytes).unwrap();
    }
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from("tests/fixtures/java/FinalWriteLinkage.java")],
        &original,
    );
    patch_fields(&original.join("FinalWriteOwner.class"), 45, false);
    patch_fields(&original.join("LegacyFinalWriter.class"), 45, false);
    patch_fields(&original.join("ModernFinalWriter.class"), 53, false);
    patch_fields(&original.join("ConstantFinalOwner.class"), 45, true);
    // Compile a real GETSTATIC, then retarget it to the interface constant.
    let caller_path = original.join("FinalWriteCaller.class");
    let mut bytes = fs::read(&caller_path).unwrap();
    replace(&mut bytes, b"ready", b"value");
    fs::write(caller_path, bytes).unwrap();
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .args(["--no-api", "--threads", "1"]),
    );
    let caller = fs::read_to_string(source.join("FinalWriteCaller.java")).unwrap();
    assert!(caller.contains("IllegalAccessError"), "{caller}");
    compile(&crate::common::sources(&source), &rebuilt);
    let metadata = sporeflower::classfile::ClassFile::parse(
        &fs::read(rebuilt.join("ConstantFinalOwner.class")).unwrap(),
    )
    .unwrap();
    let field = metadata
        .fields
        .iter()
        .find(|field| field.name == "value")
        .unwrap();
    assert_ne!(field.access_flags & 0x10, 0);
    assert!(matches!(
        metadata
            .constant_pool
            .get(field.constant_value.unwrap())
            .unwrap(),
        sporeflower::classfile::Constant::Integer(4)
    ));
    let execute = |root: &Path, action: &str, mode: i32| {
        String::from_utf8(
            checked(
                Command::new(crate::common::java())
                    .args(["-Xverify:all", "-cp"])
                    .arg(root)
                    .arg("FinalWriteLinkage")
                    .arg(action)
                    .arg(mode.to_string()),
            )
            .stdout,
        )
        .unwrap()
    };
    for (action, mode) in [
        ("direct", 0),
        ("direct", 1),
        ("direct", 3),
        ("compound", 0),
        ("compound", 1),
        ("divide", 4),
        ("increment", 0),
        ("increment", 1),
        ("decrement", 1),
        ("directStatic", 1),
        ("directStatic", 2),
        ("directStatic", 3),
        ("compoundStatic", 1),
        ("compoundStatic", 2),
        ("divideStatic", 4),
        ("incrementStatic", 1),
        ("readConstant", 1),
        ("readConstant", 2),
        ("guardedConstant", 0),
        ("guardedConstant", 1),
        ("ignoredConstant", 1),
        ("ignoredConstant", 2),
        ("compoundConstant", 1),
        ("compoundConstant", 2),
        ("compoundConstant", 3),
        ("divideConstant", 4),
        ("incrementConstant", 1),
        ("incrementConstant", 2),
        ("readInterface", 1),
        ("readInterface", 2),
        ("legacy", 1),
        ("legacyStatic", 1),
        ("modern", 1),
        ("modernStatic", 1),
    ] {
        let expected = execute(&original, action, mode);
        assert_eq!(
            execute(&rebuilt, action, mode),
            expected,
            "{action}, mode {mode}\n{caller}"
        );
    }
    assert_eq!(
        execute(&original, "direct", 0),
        "java.lang.IllegalAccessError\nreceiver;argument;\n"
    );
    assert_eq!(
        execute(&original, "compound", 0),
        "java.lang.NullPointerException\nreceiver;\n"
    );
    assert_eq!(
        execute(&original, "directStatic", 2),
        "java.lang.IllegalAccessError\nargument;\n"
    );
    assert_eq!(
        execute(&original, "divideStatic", 4),
        "java.lang.ArithmeticException\ninit;argument;\n"
    );
    assert_eq!(execute(&original, "legacy", 1), "7\n\n");
    assert_eq!(execute(&original, "legacyStatic", 1), "9\n\n");
    assert_eq!(execute(&original, "readConstant", 1), "4\nconstant-init;\n");
    assert_eq!(
        execute(&original, "compoundConstant", 1),
        "java.lang.IllegalAccessError\nconstant-init;argument;\n"
    );
    assert_eq!(
        execute(&original, "compoundConstant", 2),
        "java.lang.ExceptionInInitializerError\nconstant-init;\n"
    );
    assert_eq!(
        execute(&original, "readInterface", 1),
        "4\ninterface-init;\n"
    );
    assert_eq!(execute(&original, "guardedConstant", 0), "9\n\n");
}
