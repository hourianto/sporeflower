use crate::common::checked;
use sporeflower::{
    api::{ApiCatalog, ApiOptions},
    bytecode::{self, Operand},
    classfile::{ClassFile, Code, Constant, ConstantPool, Field, Method},
    java::{JavaOptions, decompile_class},
    naming::{NamePlan, RenameOptions},
    program::Program,
};
use std::{fs, io::Read, path::Path, process::Command};

const OWNER: &str = "vendor/MissingTable";

fn fixture(directory: &Path) -> (ClassFile, ClassFile) {
    let vendor = directory.join("vendor");
    fs::create_dir(&vendor).unwrap();
    let api = vendor.join("MissingTable.java");
    fs::write(
        &api,
        r#"
package vendor;
public class MissingTable {
    public int value;
    public static int flag;
    public MissingTable() {}
    public native void open() throws java.io.IOException;
    public native int findByName(String name);
    public native void close();
    public static native int shared();
}
"#,
    )
    .unwrap();
    let source = directory.join("InferredClient.java");
    fs::write(
        &source,
        r#"
public class InferredClient {
    public static int lookup(String name) {
        try {
            vendor.MissingTable table = new vendor.MissingTable();
            table.value = 3;
            vendor.MissingTable.flag = 4;
            table.open();
            int result = table.findByName(name);
            table.close();
            return result + table.value + vendor.MissingTable.flag + vendor.MissingTable.shared();
        } catch (Exception failure) { return -1; }
    }
    public static void checked() {
        try { new vendor.MissingTable().open(); }
        catch (java.io.IOException failure) { System.out.println("checked"); }
    }
    public static void unknown() throws java.io.IOException {
        new vendor.MissingTable().open();
    }
}
"#,
    )
    .unwrap();
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-g:none", "-d"])
            .arg(directory)
            .arg(source)
            .arg(api),
    );
    (
        ClassFile::parse(&fs::read(directory.join("InferredClient.class")).unwrap()).unwrap(),
        ClassFile::parse(&fs::read(vendor.join("MissingTable.class")).unwrap()).unwrap(),
    )
}

fn jar_entry(jar: &Path, name: &str) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(fs::File::open(jar).unwrap()).unwrap();
    let mut bytes = Vec::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    bytes
}

#[test]
fn inferred_exports_leave_semantic_resolution_and_checked_catches_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, _) = fixture(dir.path());
    let phantom = client.constant_pool.entries.len() as u16;
    client.constant_pool.entries.extend([
        Constant::Utf8("absent/PhantomImport".into()),
        Constant::Class(phantom),
    ]);
    let project = [client];
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&project, &ApiOptions::default())
        .unwrap();
    assert_eq!(api.inferred_class_count(), 1);
    assert!(!api.providers.contains_key(OWNER));
    assert!(
        api.warnings
            .iter()
            .any(|warning| warning.contains("bytecode-inferred compile stub"))
    );
    let program = Program::new(&project, &api.classes).unwrap();
    assert!(program.id(OWNER).is_none());
    assert!(program.resolve_method(OWNER, "open", "()V").is_none());
    let unknown = program
        .resolve_method("InferredClient", "unknown", "()V")
        .unwrap();
    assert!(program.throws_plan().unknown_escape(unknown));
    let names = NamePlan::build(&program, &RenameOptions::default()).unwrap();
    let decompiled = decompile_class(&program, &names, 0, &JavaOptions::default()).unwrap();
    assert_eq!(decompiled.failed_methods, 0, "{:?}", decompiled.diagnostics);
    assert!(
        !decompiled.source.contains("PhantomImport"),
        "{}",
        decompiled.source
    );
    assert!(
        decompiled.source.contains("catch (IOException "),
        "{}",
        decompiled.source
    );

    let jar = dir.path().join("api.jar");
    api.write_jar(&jar).unwrap();
    let class = ClassFile::parse(&jar_entry(&jar, "vendor/MissingTable.class")).unwrap();
    assert_eq!(class.super_name.as_deref(), Some("java/lang/Object"));
    assert!(class.interfaces.is_empty());
    assert_eq!(class.methods.len(), 5);
    assert_eq!(class.fields.len(), 2);
    for method in &class.methods {
        assert!(method.exceptions.is_empty());
        assert_eq!(method.code.as_ref().unwrap().bytes, [0x01, 0xbf]);
        assert_eq!(method.access_flags & 8 != 0, method.name == "shared");
    }
    for field in &class.fields {
        assert!(field.constant_value.is_none());
        assert_eq!(field.access_flags & 8 != 0, field.name == "flag");
    }
    let manifest = String::from_utf8(jar_entry(&jar, "META-INF/MANIFEST.MF")).unwrap();
    assert!(manifest.contains("J2ME-Stub-Fallback: true"));
    assert!(manifest.contains("J2ME-Inferred-Stub-Count: 1"));
    let metadata: serde_json::Value =
        serde_json::from_slice(&jar_entry(&jar, "META-INF/sporeflower-inferred-api.json")).unwrap();
    assert_eq!(
        metadata[OWNER]["kind"],
        "bytecode-inferred compile-only placeholder"
    );
    assert!(
        metadata[OWNER]["evidence"]
            .as_str()
            .unwrap()
            .contains("decoded new")
    );
    assert!(
        metadata[OWNER]["unknown"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "checked exceptions")
    );

    // Compile a neutral client against the export alone. The checked-catch
    // probe above intentionally cannot use an unknown real throws contract as
    // a compile guarantee; retaining its catch proves analysis was not narrowed.
    let probe = dir.path().join("CompileProbe.java");
    fs::write(
        &probe,
        r#"
public class CompileProbe {
    public static int lookup(String name) {
        try {
            vendor.MissingTable table = new vendor.MissingTable();
            table.value = 3;
            vendor.MissingTable.flag = 4;
            table.open();
            int result = table.findByName(name);
            table.close();
            return result + table.value + vendor.MissingTable.flag + vendor.MissingTable.shared();
        } catch (Exception failure) { return -1; }
    }
}
"#,
    )
    .unwrap();
    let output = dir.path().join("compiled");
    fs::create_dir(&output).unwrap();
    let mut compiler = if let Some(legacy) = crate::common::legacy_javac() {
        crate::common::legacy_command(legacy, &jar)
    } else {
        let mut command = Command::new(crate::common::javac());
        command.args(["--release", "8"]);
        command
    };
    checked(
        compiler
            .arg("-classpath")
            .arg(&jar)
            .arg("-d")
            .arg(&output)
            .arg(probe),
    );
    assert!(output.join("CompileProbe.class").exists());
}

fn reference(client: &mut ClassFile, name: &str, descriptor: &str, opcode: u8) {
    let pool = &mut client.constant_pool.entries;
    let start = pool.len() as u16;
    pool.extend([
        Constant::Utf8(OWNER.into()),
        Constant::Class(start),
        Constant::Utf8(name.into()),
        Constant::Utf8(descriptor.into()),
        Constant::NameAndType {
            name_index: start + 2,
            descriptor_index: start + 3,
        },
        if opcode == 0xb9 {
            Constant::InterfaceMethodref {
                class_index: start + 1,
                name_and_type_index: start + 4,
            }
        } else {
            Constant::Methodref {
                class_index: start + 1,
                name_and_type_index: start + 4,
            }
        },
    ]);
    let index = start + 5;
    let mut bytes = vec![opcode, (index >> 8) as u8, index as u8];
    if opcode == 0xb9 {
        bytes.extend([1, 0]);
    }
    bytes.push(0xb1);
    client.methods.push(Method {
        access_flags: 9,
        name: "conflict".into(),
        descriptor: "()V".into(),
        exceptions: vec![],
        synthetic: false,
        code: Some(Code {
            max_stack: 1,
            max_locals: 0,
            bytes,
            exception_table: vec![],
            line_numbers: vec![],
            local_variables: vec![],
        }),
    });
}

#[test]
fn inference_rejects_conflicting_members() {
    let dir = tempfile::tempdir().unwrap();
    let (client, _) = fixture(dir.path());
    for (name, descriptor, opcode, reason) in [
        ("open", "()V", 0xb8, "conflicting descriptor or staticness"),
        (
            "open",
            "()V",
            0xb9,
            "conflicting class evidence and interface invocation",
        ),
        (
            "findByName",
            "(Ljava/lang/String;)Z",
            0xb6,
            "conflicting descriptor or staticness",
        ),
    ] {
        let mut conflicting = client.clone();
        reference(&mut conflicting, name, descriptor, opcode);
        let api = ApiCatalog::bundled()
            .unwrap()
            .select(&[conflicting], &ApiOptions::default())
            .unwrap();
        assert_eq!(api.inferred_class_count(), 0);
        assert!(
            api.warnings.iter().any(|warning| warning.contains(reason)),
            "{:?}",
            api.warnings
        );
    }
}

#[test]
fn inference_accepts_decoded_class_invocations_and_ignores_unused_pool_entries() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, _) = fixture(dir.path());
    for method in &mut client.methods {
        let Some(code) = &mut method.code else {
            continue;
        };
        for instruction in bytecode::decode(&code.bytes).unwrap() {
            if instruction.opcode == 0xbb {
                let Operand::Constant(index) = instruction.operand else {
                    unreachable!()
                };
                if client.constant_pool.class_name(index).unwrap() == OWNER {
                    code.bytes[instruction.pc as usize..instruction.pc as usize + 3].fill(0);
                }
            }
        }
    }
    let catalog = ApiCatalog::bundled().unwrap();
    let api = catalog
        .select(&[client.clone()], &ApiOptions::default())
        .unwrap();
    assert_eq!(api.inferred_class_count(), 1);
    for method in &mut client.methods {
        let Some(code) = &mut method.code else {
            continue;
        };
        for instruction in bytecode::decode(&code.bytes).unwrap() {
            if matches!(instruction.opcode, 0xb6..=0xb8) {
                code.bytes[instruction.pc as usize..instruction.pc as usize + 3].fill(0);
            }
        }
    }
    let api = catalog
        .select(&[client.clone()], &ApiOptions::default())
        .unwrap();
    assert_eq!(api.inferred_class_count(), 0);
    assert!(
        api.warnings
            .iter()
            .any(|warning| warning.contains("no decoded allocation or class invocation"))
    );
    client.methods.clear();
    let api = catalog.select(&[client], &ApiOptions::default()).unwrap();
    assert_eq!(api.inferred_class_count(), 0);
    assert!(api.warnings.is_empty(), "{:?}", api.warnings);
}

#[test]
fn explicit_classpath_is_consulted_before_inferred_exports() {
    let dir = tempfile::tempdir().unwrap();
    let (client, library) = fixture(dir.path());
    let project = [client];
    let api = ApiCatalog::bundled()
        .unwrap()
        .select_with_classpath(
            &project,
            std::slice::from_ref(&library),
            &ApiOptions::default(),
        )
        .unwrap();
    assert_eq!(api.inferred_class_count(), 0);
    assert!(api.warnings.is_empty(), "{:?}", api.warnings);
    let jar = dir.path().join("api.jar");
    api.write_jar(&jar).unwrap();
    let mut archive = zip::ZipArchive::new(fs::File::open(jar).unwrap()).unwrap();
    assert!(archive.by_name("vendor/MissingTable.class").is_err());
    let mut libraries = api.classes;
    libraries.push(library);
    let program = Program::new(&project, &libraries).unwrap();
    let method = program.resolve_method(OWNER, "open", "()V").unwrap();
    assert_eq!(program.method_throws(method), ["java/io/IOException"]);

    let cli_jar = dir.path().join("cli-api.jar");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(dir.path().join("InferredClient.class"))
            .arg("--classpath")
            .arg(dir.path().join("vendor/MissingTable.class"))
            .arg("--output")
            .arg(dir.path().join("source"))
            .arg("--stubs")
            .arg(&cli_jar),
    );
    let manifest = String::from_utf8(jar_entry(&cli_jar, "META-INF/MANIFEST.MF")).unwrap();
    assert!(manifest.contains("J2ME-Inferred-Stub-Count: 0"));
}

fn derived_class(name: &str, parent: &str) -> ClassFile {
    let mut constant_pool = ConstantPool::default();
    constant_pool.entries = vec![
        Constant::Unusable,
        Constant::Utf8(parent.into()),
        Constant::Class(1),
        Constant::Utf8("<init>".into()),
        Constant::Utf8("()V".into()),
        Constant::NameAndType {
            name_index: 3,
            descriptor_index: 4,
        },
        Constant::Methodref {
            class_index: 2,
            name_and_type_index: 5,
        },
    ];
    ClassFile {
        minor_version: 3,
        major_version: 45,
        access_flags: 0x21,
        name: name.into(),
        super_name: Some(parent.into()),
        interfaces: vec![],
        constant_pool,
        fields: vec![],
        methods: vec![Method {
            access_flags: 1,
            name: "<init>".into(),
            descriptor: "()V".into(),
            code: Some(Code {
                max_stack: 1,
                max_locals: 1,
                bytes: vec![0x2a, 0xb7, 0, 6, 0xb1],
                exception_table: vec![],
                line_numbers: vec![],
                local_variables: vec![],
            }),
            exceptions: vec![],
            synthetic: false,
        }],
        source_file: None,
        inner_classes: vec![],
        warnings: vec![],
    }
}

#[test]
fn effective_sdk_declarations_supply_callback_and_signature_dependencies() {
    let parent = "vendor/SDKParent";
    // The application names only its supplied parent and its constructor. Its
    // pool contains no Canvas or signature dependency that could seed selection.
    let child = derived_class("SDKChild", parent);
    let mut sdk = derived_class(parent, "javax/microedition/lcdui/Canvas");
    sdk.access_flags |= 0x400;
    sdk.interfaces
        .push("javax/microedition/lcdui/CommandListener".into());
    sdk.fields.push(Field {
        access_flags: 1,
        name: "image".into(),
        descriptor: "Ljavax/microedition/lcdui/Image;".into(),
        constant_value: None,
        synthetic: false,
    });
    sdk.methods.push(Method {
        access_flags: 0x101,
        name: "display".into(),
        descriptor: "(Ljavax/microedition/media/Player;)Ljavax/microedition/lcdui/Display;".into(),
        code: None,
        exceptions: vec!["javax/microedition/media/MediaException".into()],
        synthetic: false,
    });
    // These implementation-only references must neither request inferred stubs
    // nor force the project's CLDC 1.0 boot preference to CLDC 1.1.
    sdk.constant_pool.entries.push(Constant::Float(1.0));
    let class_index = sdk.constant_pool.entries.len() as u16 + 1;
    reference(&mut sdk, "<init>", "()V", 0xb7);
    let code = sdk.methods.last_mut().unwrap().code.as_mut().unwrap();
    code.bytes.splice(
        0..0,
        [0xbb, (class_index >> 8) as u8, class_index as u8, 0x59],
    );
    code.bytes.insert(code.bytes.len() - 1, 0x57);
    code.max_stack = 2;

    let superseded = derived_class(parent, "com/samsung/util/AudioClip");
    let project = [child.clone()];
    let classpath = [superseded, sdk.clone()];
    let options = ApiOptions {
        configuration: Some("CLDC-1.0".into()),
        ..Default::default()
    };
    let catalog = ApiCatalog::bundled().unwrap();
    let api = catalog
        .select_with_classpath(&project, &classpath, &options)
        .unwrap();
    assert!(api.warnings.is_empty(), "{:?}", api.warnings);
    assert_eq!(api.inferred_class_count(), 0);
    assert_eq!(api.providers["java/lang/Object"], "cldcapi10-2.0.4");
    assert!(!api.providers.contains_key("com/samsung/util/AudioClip"));
    for name in [
        "javax/microedition/lcdui/Canvas",
        "javax/microedition/lcdui/CommandListener",
        "javax/microedition/lcdui/Image",
        "javax/microedition/lcdui/Display",
        "javax/microedition/media/Player",
        "javax/microedition/media/MediaException",
    ] {
        assert!(
            api.classes.iter().any(|class| class.name == name),
            "missing {name}"
        );
    }
    let mut libraries = api.classes;
    libraries.push(sdk.clone());
    let program = Program::new(&project, &libraries).unwrap();
    let callback = program
        .resolve_method(
            "SDKChild",
            "paint",
            "(Ljavax/microedition/lcdui/Graphics;)V",
        )
        .unwrap();
    assert_eq!(
        program.class(callback.class).name,
        "javax/microedition/lcdui/Canvas"
    );
    assert!(program.id(OWNER).is_none());

    // An application declaration also supersedes the last SDK entry, including
    // all of that entry's otherwise visible declaration dependencies.
    let project = [child, derived_class(parent, "java/lang/Object")];
    let api = catalog
        .select_with_classpath(&project, &[sdk], &options)
        .unwrap();
    assert!(
        !api.providers
            .contains_key("javax/microedition/lcdui/Canvas")
    );
    assert_eq!(api.inferred_class_count(), 0);
}

#[test]
fn static_only_api_exports_need_no_constructor_and_remain_semantically_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, _) = fixture(dir.path());
    client.methods.clear();
    reference(&mut client, "send", "(Ljava/lang/String;)V", 0xb8);
    let project = [client];
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&project, &ApiOptions::default())
        .unwrap();
    assert_eq!(api.inferred_class_count(), 1);
    let program = Program::new(&project, &api.classes).unwrap();
    assert!(program.id(OWNER).is_none());
    let jar = dir.path().join("api.jar");
    api.write_jar(&jar).unwrap();
    let class = ClassFile::parse(&jar_entry(&jar, "vendor/MissingTable.class")).unwrap();
    assert_eq!(class.methods.len(), 1);
    assert_eq!(class.methods[0].name, "send");
    assert_eq!(class.methods[0].access_flags & 8, 8);
    let probe = dir.path().join("StaticProbe.java");
    fs::write(
        &probe,
        "public class StaticProbe { public void send() { vendor.MissingTable.send(\"hello\"); } }",
    )
    .unwrap();
    let output = dir.path().join("compiled");
    fs::create_dir(&output).unwrap();
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-classpath"])
            .arg(&jar)
            .arg("-d")
            .arg(&output)
            .arg(&probe),
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&jar_entry(&jar, "META-INF/sporeflower-inferred-api.json")).unwrap();
    assert!(
        metadata[OWNER]["evidence"]
            .as_str()
            .unwrap()
            .contains("decoded class invocation")
    );
}

#[test]
fn descriptor_only_types_have_opaque_export_shapes_without_semantic_declarations() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, _) = fixture(dir.path());
    client.fields.push(Field {
        name: "opaque".into(),
        descriptor: "Lvendor/Opaque;".into(),
        access_flags: 1,
        constant_value: None,
        synthetic: false,
    });
    reference(&mut client, "result", "()Lvendor/Result;", 0xb6);
    let catalog = ApiCatalog::bundled().unwrap();
    let api = catalog
        .select(&[client.clone()], &ApiOptions::default())
        .unwrap();
    assert_eq!(api.inferred_class_count(), 3);
    assert!(
        !api.classes
            .iter()
            .any(|class| matches!(class.name.as_str(), "vendor/Opaque" | "vendor/Result"))
    );
    let jar = dir.path().join("api.jar");
    api.write_jar(&jar).unwrap();
    for name in ["vendor/Opaque", "vendor/Result"] {
        let class = ClassFile::parse(&jar_entry(&jar, &format!("{name}.class"))).unwrap();
        assert!(class.methods.is_empty() && class.fields.is_empty());
        let metadata: serde_json::Value =
            serde_json::from_slice(&jar_entry(&jar, "META-INF/sporeflower-inferred-api.json"))
                .unwrap();
        assert!(
            metadata[name]["unknown"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "class kind")
        );
    }
    let source = dir.path().join("OpaqueProbe.java");
    fs::write(&source, "public class OpaqueProbe { private vendor.Opaque value = null; public vendor.Result[] values; }").unwrap();
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-classpath"])
            .arg(&jar)
            .arg(&source),
    );
    // A descriptor cannot justify a class shape when Java requires an interface.
    client.interfaces.push("vendor/Opaque".into());
    let api = catalog.select(&[client], &ApiOptions::default()).unwrap();
    assert_eq!(api.inferred_class_count(), 2);
}

#[test]
fn incomplete_superclass_lookup_keeps_inherited_calls_and_fields_unknown() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    fs::create_dir(&original).unwrap();
    let parent = work.path().join("Parent.java");
    fs::write(&parent, "package vendor; public class Parent { public int value = 7; public static int inherited() { return 11; } }").unwrap();
    let child = work.path().join("Child.java");
    fs::write(&child, "public class Child extends vendor.Parent { public int check() { return value + inherited(); } public static void main(String[] args) { System.out.println(new Child().check()); } }").unwrap();
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-d"])
            .arg(&original)
            .args([parent, child]),
    );
    let classes = [ClassFile::parse(&fs::read(original.join("Child.class")).unwrap()).unwrap()];
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    assert_eq!(api.inferred_class_count(), 1);
    let program = Program::new(&classes, &api.classes).unwrap();
    assert!(
        program
            .resolve_method("Child", "inherited", "()I")
            .is_none()
    );
    assert!(program.resolve_field("Child", "value", "I").is_none());
    let names = NamePlan::build(&program, &RenameOptions::default()).unwrap();
    let rendered = decompile_class(&program, &names, 0, &JavaOptions::default()).unwrap();
    assert_eq!(rendered.failed_methods, 0, "{:?}", rendered.diagnostics);
    let jar = work.path().join("api.jar");
    api.write_jar(&jar).unwrap();
    let inferred = ClassFile::parse(&jar_entry(&jar, "vendor/Parent.class")).unwrap();
    assert!(
        inferred
            .methods
            .iter()
            .any(|method| method.name == "inherited" && method.access_flags & 8 != 0)
    );
    assert!(inferred.fields.iter().any(|field| field.name == "value"));
    let source = work.path().join("Child.java");
    fs::write(&source, rendered.source).unwrap();
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir(&rebuilt).unwrap();
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-classpath"])
            .arg(&jar)
            .arg("-d")
            .arg(&rebuilt)
            .arg(&source),
    );
    fs::create_dir(rebuilt.join("vendor")).unwrap();
    fs::copy(
        original.join("vendor/Parent.class"),
        rebuilt.join("vendor/Parent.class"),
    )
    .unwrap();
    assert_eq!(crate::common::execute(&rebuilt, "Child"), b"18\n");
}
