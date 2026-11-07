use sporeflower::{
    api::{ApiCatalog, ApiOptions},
    classfile::{ClassFile, Code, Constant, ConstantPool, Field, Method},
};
use std::{fs, io::Read};

fn project(name: &str, parent: &str) -> ClassFile {
    let mut constant_pool = ConstantPool::default();
    constant_pool.entries.push(Constant::Unusable);
    ClassFile {
        minor_version: 3,
        major_version: 45,
        access_flags: 0x21,
        name: name.into(),
        super_name: Some(parent.into()),
        interfaces: vec![],
        constant_pool,
        fields: vec![],
        methods: vec![],
        source_file: None,
        inner_classes: vec![],
        warnings: vec![],
    }
}

fn method_reference(class: &mut ClassFile, owner: &str, name: &str, descriptor: &str, opcode: u8) {
    let cp = &mut class.constant_pool.entries;
    let start = cp.len() as u16;
    cp.extend([
        Constant::Utf8(owner.into()),
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
    class.methods.push(Method {
        access_flags: 9,
        name: format!("probe{}", class.methods.len()),
        descriptor: "()V".into(),
        code: Some(Code {
            max_stack: 4,
            max_locals: 0,
            bytes,
            exception_table: vec![],
            line_numbers: vec![],
            local_variables: vec![],
        }),
        exceptions: vec![],
        synthetic: false,
    });
}

#[test]
fn loads_every_catalog_and_exports_parseable_stubs() {
    let catalog = ApiCatalog::bundled().unwrap();
    let selected = catalog
        .select(
            &[],
            &ApiOptions {
                include_all: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(selected.warnings.is_empty());
    for class in &selected.classes {
        for method in &class.methods {
            if let Some(code) = &method.code {
                assert_eq!(code.bytes, [0x01, 0xbf], "{}.{}", class.name, method.name);
            } else {
                assert_ne!(
                    method.access_flags & (0x0400 | 0x0100),
                    0,
                    "{}.{}",
                    class.name,
                    method.name
                );
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let jar = dir.path().join("api.jar");
    selected.write_jar(&jar).unwrap();
    let mut archive = zip::ZipArchive::new(fs::File::open(&jar).unwrap()).unwrap();
    let mut class_count = 0;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        let name = entry.name().unwrap().into_owned();
        if !name.ends_with(".class") {
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        let class = ClassFile::parse(&bytes).unwrap();
        assert_eq!(format!("{}.class", class.name), name);
        class_count += 1;
    }
    assert_eq!(class_count, selected.classes.len());
}

#[test]
fn unsupported_return_descriptor_does_not_invent_an_api_variant() {
    let catalog = ApiCatalog::bundled().unwrap();
    let owner = "com/motorola/io/file/FileConnection";
    for descriptor in ["()V", "()Z"] {
        let mut class = project("Example", "java/lang/Object");
        method_reference(&mut class, owner, "create", descriptor, 0xb9);
        let selected = catalog.select(&[class], &ApiOptions::default()).unwrap();
        assert_eq!(selected.providers[owner], "motorola-io-file");
        let warnings: Vec<_> = if descriptor == "()V" {
            vec![]
        } else {
            vec![format!(
                "selected API has no instance method {owner}.create()Z"
            )]
        };
        assert_eq!(selected.warnings, warnings);
        assert_eq!(selected.inferred_class_count(), 0);
        let api = selected
            .classes
            .iter()
            .find(|class| class.name == owner)
            .unwrap();
        let methods: Vec<_> = api
            .methods
            .iter()
            .filter(|method| method.name == "create")
            .collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].descriptor, "()V");
        assert_eq!(methods[0].exceptions, ["java/io/IOException"]);
    }
}

#[test]
fn opcode_staticness_selects_matching_variant() {
    let catalog = ApiCatalog::bundled().unwrap();
    let owner = "com/samsung/util/LCDLight";
    for (opcode, provider) in [(0xb8, "kemulator-extensions"), (0xb6, "samsung-api")] {
        let mut class = project("Example", "java/lang/Object");
        method_reference(&mut class, owner, "isSupported", "()Z", opcode);
        let selected = catalog.select(&[class], &ApiOptions::default()).unwrap();
        assert_eq!(selected.providers[owner], provider);
        assert!(selected.warnings.is_empty(), "{:?}", selected.warnings);
    }
}

#[test]
fn inherited_project_reference_reaches_api_provider() {
    let catalog = ApiCatalog::bundled().unwrap();
    let owner = "com/samsung/util/LCDLight";
    let mut class = project("Example", owner);
    method_reference(&mut class, "Example", "isSupported", "()Z", 0xb8);
    let selected = catalog.select(&[class], &ApiOptions::default()).unwrap();
    assert_eq!(selected.providers[owner], "kemulator-extensions");
    assert!(selected.warnings.is_empty(), "{:?}", selected.warnings);
}

#[test]
fn cldc10_preference_yields_to_actual_floating_point_requirements() {
    let catalog = ApiCatalog::bundled().unwrap();
    let options = ApiOptions {
        configuration: Some("CLDC-1.0".into()),
        ..Default::default()
    };
    let mut class = project("Example", "java/lang/Object");
    let selected = catalog.select(&[class.clone()], &options).unwrap();
    assert_eq!(selected.providers["java/lang/Object"], "cldcapi10-2.0.4");
    class.constant_pool.entries.push(Constant::Float(1.5));
    let selected = catalog.select(&[class], &options).unwrap();
    assert_eq!(selected.providers["java/lang/Object"], "cldcapi11-2.0.4");
}

#[test]
fn project_class_is_not_shadowed_by_bundled_api() {
    let catalog = ApiCatalog::bundled().unwrap();
    let class = project("javax/microedition/lcdui/Canvas", "java/lang/Object");
    let selected = catalog.select(&[class], &ApiOptions::default()).unwrap();
    assert!(
        !selected
            .providers
            .contains_key("javax/microedition/lcdui/Canvas")
    );
    assert!(
        !selected
            .classes
            .iter()
            .any(|class| class.name == "javax/microedition/lcdui/Canvas")
    );
}

#[test]
fn selected_boot_api_always_contains_language_exception_roots() {
    let selected = ApiCatalog::bundled()
        .unwrap()
        .select(&[], &ApiOptions::default())
        .unwrap();
    for name in [
        "java/lang/Object",
        "java/lang/Class",
        "java/lang/String",
        "java/lang/Throwable",
        "java/lang/Exception",
        "java/lang/RuntimeException",
        "java/lang/Error",
        "java/lang/NoClassDefFoundError",
    ] {
        assert!(
            selected.classes.iter().any(|class| class.name == name),
            "missing language root {name}"
        );
    }
}

#[test]
fn vendor_array_type_exports_its_complete_documented_contract() {
    let owner = "com/samsung/util/AudioClip";
    let mut client = project("AudioClient", "java/lang/Object");
    // A declaration alone must select its API, without a method reference or
    // CONSTANT_Class entry supplying the same name independently.
    client.fields.push(Field {
        access_flags: 2,
        name: "clips".into(),
        descriptor: format!("[L{owner};"),
        constant_value: None,
        synthetic: false,
    });
    let selected = ApiCatalog::bundled()
        .unwrap()
        .select(&[client], &ApiOptions::default())
        .unwrap();
    assert!(selected.warnings.is_empty(), "{:?}", selected.warnings);
    assert_eq!(selected.providers[owner], "samsung-audioclip");
    let dir = tempfile::tempdir().unwrap();
    let jar = dir.path().join("api.jar");
    selected.write_jar(&jar).unwrap();
    let mut archive = zip::ZipArchive::new(fs::File::open(&jar).unwrap()).unwrap();
    let mut bytes = Vec::new();
    archive
        .by_name(&format!("{owner}.class"))
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    let declaration = ClassFile::parse(&bytes).unwrap();
    assert_eq!(declaration.super_name.as_deref(), Some("java/lang/Object"));
    assert_eq!(declaration.access_flags, 0x21);
    assert_eq!(declaration.fields.len(), 3);
    for (name, value) in [("TYPE_MMF", 1), ("TYPE_MP3", 2), ("TYPE_MIDI", 3)] {
        let field = declaration
            .fields
            .iter()
            .find(|field| field.name == name)
            .unwrap();
        assert_eq!(field.access_flags, 0x19);
        assert_eq!(field.descriptor, "I");
        assert!(matches!(
            declaration.constant_pool.entries[usize::from(field.constant_value.unwrap())],
            Constant::Integer(actual) if actual == value
        ));
    }
    assert_eq!(declaration.methods.len(), 7);
    for (name, descriptor, flags, exceptions) in [
        (
            "<init>",
            "(ILjava/lang/String;)V",
            1,
            vec!["java/io/IOException"],
        ),
        ("<init>", "(I[BII)V", 1, vec![]),
        ("isSupported", "()Z", 9, vec![]),
        ("pause", "()V", 1, vec![]),
        ("play", "(II)V", 1, vec![]),
        ("resume", "()V", 1, vec![]),
        ("stop", "()V", 1, vec![]),
    ] {
        let method = declaration
            .methods
            .iter()
            .find(|method| method.name == name && method.descriptor == descriptor)
            .unwrap();
        assert_eq!(method.access_flags, flags);
        assert_eq!(method.exceptions, exceptions);
        assert_eq!(method.code.as_ref().unwrap().bytes, [0x01, 0xbf]);
    }
    assert!(archive.by_name("java/io/IOException.class").is_ok());

    let Some(legacy) = crate::common::legacy_javac() else {
        return;
    };
    let source = dir.path().join("AudioClient.java");
    fs::write(
        &source,
        r#"
import com.samsung.util.AudioClip;
public class AudioClient {
    private AudioClip[] clips;
    public AudioClient(byte[] data, String path) {
        clips = new AudioClip[2];
        clips[0] = new AudioClip(AudioClip.TYPE_MMF, data, 0, data.length);
        try { clips[1] = new AudioClip(AudioClip.TYPE_MIDI, path); }
        catch (java.io.IOException failure) { clips[1] = clips[0]; }
    }
    public void control(int repeat, int volume) {
        if (AudioClip.isSupported()) {
            clips[0].play(repeat, volume);
            clips[0].pause();
            clips[0].resume();
            clips[0].stop();
        }
    }
    public int alternateType() { return AudioClip.TYPE_MP3; }
}
"#,
    )
    .unwrap();
    let output = crate::common::legacy_command(legacy, &jar)
        .arg("-d")
        .arg(dir.path())
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let compiled =
        ClassFile::parse(&fs::read(dir.path().join("AudioClient.class")).unwrap()).unwrap();
    assert_eq!((compiled.major_version, compiled.minor_version), (45, 3));
}

#[test]
fn exported_stubs_compile_a_midlet_without_sdk_installation() {
    let catalog = ApiCatalog::bundled().unwrap();
    let class = project("Example", "javax/microedition/midlet/MIDlet");
    let selected = catalog.select(&[class], &ApiOptions::default()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let jar = dir.path().join("api.jar");
    selected.write_jar(&jar).unwrap();
    let source = dir.path().join("Example.java");
    fs::write(&source, "public final class Example extends javax.microedition.midlet.MIDlet { protected void startApp() {} protected void pauseApp() {} protected void destroyApp(boolean unconditional) {} public String property() { return getAppProperty(\"MIDlet-Name\"); } }").unwrap();
    let output = std::process::Command::new(crate::common::javac())
        .args(["--release", "8", "-classpath"])
        .arg(&jar)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(dir.path().join("Example.class").exists());
}

#[test]
fn vendor_service_declarations_preserve_kinds_signatures_and_checked_exceptions() {
    let mut client = project("VendorServices", "java/lang/Object");
    for (owner, name, descriptor, opcode) in [
        ("com/nokia/mid/ui/CategoryBar", "setMode", "(I)V", 0xb6),
        ("com/nokia/mid/ui/CategoryBar", "getMode", "()I", 0xb6),
        (
            "com/siemens/mp/media/control/VolumeControl",
            "setLevel",
            "(I)I",
            0xb9,
        ),
        (
            "com/siemens/mp/io/File",
            "list",
            "(Ljava/lang/String;Z)[Ljava/lang/String;",
            0xb8,
        ),
        (
            "com/nokia/notifications/NotificationSessionFactory",
            "openSession",
            "(Ljavax/microedition/midlet/MIDlet;Ljava/lang/String;Ljava/lang/String;Lcom/nokia/notifications/NotificationSessionListener;)Lcom/nokia/notifications/NotificationSession;",
            0xb8,
        ),
        (
            "com/nokia/mid/payment/IAPClientPaymentManager",
            "getIAPClientPaymentManager",
            "()Lcom/nokia/mid/payment/IAPClientPaymentManager;",
            0xb8,
        ),
    ] {
        method_reference(&mut client, owner, name, descriptor, opcode);
    }
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&[client], &ApiOptions::default())
        .unwrap();
    assert!(api.warnings.is_empty(), "{:?}", api.warnings);
    assert_eq!(api.inferred_class_count(), 0);
    let get = |name: &str| api.classes.iter().find(|class| class.name == name).unwrap();
    let session = get("com/nokia/notifications/NotificationSession");
    assert_ne!(session.access_flags & 0x200, 0);
    assert!(
        session
            .methods
            .iter()
            .find(|method| method.name == "registerApplication")
            .unwrap()
            .exceptions
            .contains(&"com/nokia/notifications/NotificationException".into())
    );
    assert_eq!(
        get("com/nokia/notifications/NotificationException")
            .super_name
            .as_deref(),
        Some("java/lang/Exception")
    );
    assert_eq!(
        get("com/nokia/notifications/NotificationSessionListener")
            .methods
            .len(),
        3
    );
    assert_eq!(
        get("com/nokia/mid/payment/IAPClientPaymentListener")
            .methods
            .len(),
        6
    );
    assert_eq!(
        get("com/nokia/mid/payment/IAPClientPaymentManager")
            .methods
            .iter()
            .find(|method| method.name == "getIAPClientPaymentManager")
            .unwrap()
            .exceptions,
        ["com/nokia/mid/payment/IAPClientPaymentException"]
    );
    let file = get("com/siemens/mp/io/File");
    assert_eq!(api.providers[&file.name], "siemens-extended-api");
    assert_eq!(file.fields.len(), 3);
    assert!(file.fields.iter().all(|field| field.access_flags == 0x19));
    for method in &file.methods {
        let throws = !matches!(
            method.name.as_str(),
            "<init>" | "buildPath" | "checkFileName" | "delete" | "rename"
        );
        let expected: Vec<_> = if throws {
            vec!["java/io/IOException"]
        } else {
            vec![]
        };
        assert_eq!(
            method.exceptions, expected,
            "{}{}",
            method.name, method.descriptor
        );
    }
    assert_eq!(
        get("com/siemens/mp/media/control/VolumeControl").interfaces,
        ["com/siemens/mp/media/Control"]
    );
}
