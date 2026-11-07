use crate::common;
use sporeflower::{
    classfile::ClassFile,
    input::Input,
    mappings::{MappingPlan, MappingSource, parse_sources},
    naming::{NameOverrides, NamePlan, RenameOptions},
    program::{MemberId, Program},
    remap::{remap_jar, remap_jar_with_mappings},
};
use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

fn compile(work: &Path, sources: &[(&str, &str)]) -> PathBuf {
    let source = work.join("source");
    let output = work.join("classes");
    fs::create_dir_all(&source).unwrap();
    let mut files = Vec::new();
    for (name, text) in sources {
        let path = source.join(name);
        fs::write(&path, text).unwrap();
        files.push(path);
    }
    fs::create_dir_all(&output).unwrap();
    common::checked(
        common::compile_command()
            .arg("-g")
            .arg("-d")
            .arg(&output)
            .args(&files),
    );
    output
}

fn mappings(program: &Program<'_>, source: &str) -> MappingPlan {
    parse_sources(program, &[MappingSource::new("remap.map", source)]).unwrap()
}

fn jar(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut entries = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        let name = entry.name().unwrap().into_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        entries.insert(name, bytes);
    }
    entries
}

#[test]
fn remapped_jar_links_original_hierarchy_and_separates_constant_pool_roles() {
    let work = tempfile::tempdir().unwrap();
    let classes = compile(
        work.path(),
        &[
            (
                "Main.java",
                r#"package old;
public class Main {
  public static Target target;
  public static void main(String[] args) throws Exception {
    Child child = new Child();
    System.out.println(child.ping() + new Other().ping() + child.value);
    System.out.println("ping:old/Target:Lold/Target;");
    System.out.println("old.Target");
    System.out.println(Class.forName("old.Target").getName());
    System.out.println(Main.class.getResourceAsStream("data.bin").read());
    System.out.println((int) "\u0000\ud800".charAt(1));
  }
}"#,
            ),
            (
                "Parent.java",
                "package old; public class Parent { public int value=3; public int ping(){return 1;} }",
            ),
            (
                "Child.java",
                "package old; public class Child extends Parent {}",
            ),
            (
                "Other.java",
                "package old; public class Other { public int ping(){return 2;} }",
            ),
            ("Target.java", "package old; public class Target {}"),
        ],
    );
    fs::write(classes.join("old/data.bin"), [71]).unwrap();
    fs::create_dir_all(classes.join("META-INF")).unwrap();
    fs::write(classes.join("META-INF/MANIFEST.MF"), b"Manifest-Version: 1.0\r\nMain-Class: old.Main\r\nMIDlet-1: Title, icon.png, old.Main\r\n\r\nName: old/Main.class\r\nSHA-256-Digest: invalid\r\n\r\n").unwrap();
    fs::write(classes.join("META-INF/OLD.SF"), b"invalid signature").unwrap();
    let input = Input::read_for_remap(&classes).unwrap();
    let program = Program::from_input(&input, &[]).unwrap();
    let mut overrides = NameOverrides::default();
    for id in 0..program.own_count() {
        overrides.classes.insert(
            id,
            format!(
                "newpkg/{}",
                program.class(id).name.rsplit('/').next().unwrap()
            ),
        );
    }
    let parent = program.resolve_method("old/Parent", "ping", "()I").unwrap();
    let other = program.resolve_method("old/Other", "ping", "()I").unwrap();
    let field = program.resolve_field("old/Parent", "value", "I").unwrap();
    overrides.methods.insert(parent, "answer".into());
    overrides.methods.insert(other, "otherAnswer".into());
    overrides.fields.insert(field, "number".into());
    let names =
        NamePlan::build_with_overrides(&program, &RenameOptions::default(), &overrides).unwrap();
    let output = work.path().join("mapped.jar");
    let report = remap_jar(&output, &program, &names).unwrap();
    assert_eq!(report.classes, 5);
    let entries = jar(&output);
    assert_eq!(entries["old/data.bin"], [71]);
    assert_eq!(entries["newpkg/data.bin"], [71]);
    assert!(!entries.contains_key("META-INF/OLD.SF"));
    let manifest = std::str::from_utf8(&entries["META-INF/MANIFEST.MF"]).unwrap();
    assert!(manifest.contains("Main-Class: newpkg.Main"));
    assert!(manifest.contains("MIDlet-1: Title, icon.png, newpkg.Main"));
    assert!(!manifest.contains("Digest"));
    assert_eq!(
        common::execute(&output, "newpkg.Main"),
        b"6\nping:old/Target:Lold/Target;\nold.Target\nnewpkg.Target\n71\n55296\n"
    );
    for original in input.classes() {
        let mapped =
            ClassFile::parse(&entries[&format!("{}.class", names.class_name_for(&original.name))])
                .unwrap();
        assert_eq!(original.major_version, mapped.major_version);
        assert_eq!(original.access_flags, mapped.access_flags);
        for (original, mapped) in original.methods.iter().zip(&mapped.methods) {
            if let (Some(before), Some(after)) = (&original.code, &mapped.code) {
                assert_eq!(before.bytes.len(), after.bytes.len());
                assert_eq!(before.exception_table.len(), after.exception_table.len());
                assert_eq!(before.line_numbers.len(), after.line_numbers.len());
            }
        }
    }
}

#[test]
fn contracted_constant_fields_and_parameter_debug_names_are_remapped() {
    let work = tempfile::tempdir().unwrap();
    let classes = compile(
        work.path(),
        &[(
            "A.java",
            "package old; public class A { public static final String NAME=\"old.A\"; public static void keep(long first, A second) { System.out.println(second); } }",
        )],
    );
    let input = Input::read_for_remap(&classes).unwrap();
    let program = Program::from_input(&input, &[]).unwrap();
    let owner = program.id("old/A").unwrap();
    let method = program
        .resolve_method("old/A", "keep", "(JLold/A;)V")
        .unwrap();
    let mut mappings = mappings(
        &program,
        "package old; class A { @ClassName String NAME /* was NAME */; }",
    );
    mappings
        .names
        .classes
        .insert(owner, "newpkg/Renamed".into());
    mappings
        .names
        .parameters
        .insert(method, vec!["wide".into(), "renamed".into()]);
    let names =
        NamePlan::build_with_overrides(&program, &RenameOptions::default(), &mappings.names)
            .unwrap();
    let output = work.path().join("mapped.jar");
    remap_jar_with_mappings(&output, &program, &names, &mappings).unwrap();
    let mapped = ClassFile::parse(&jar(&output)["newpkg/Renamed.class"]).unwrap();
    assert_eq!(
        mapped
            .constant_pool
            .string_utf16(mapped.fields[0].constant_value.unwrap())
            .unwrap(),
        "newpkg.Renamed".encode_utf16().collect::<Vec<_>>()
    );
    let method = mapped
        .methods
        .iter()
        .find(|method| method.name == "keep")
        .unwrap();
    let locals = &method.code.as_ref().unwrap().local_variables;
    assert!(
        locals
            .iter()
            .any(|local| local.index == 0 && local.name == "wide")
    );
    assert!(locals.iter().any(|local| local.index == 2
        && local.name == "renamed"
        && local.descriptor == "Lnewpkg/Renamed;"));
}

#[test]
fn shared_reflection_values_translate_at_calls_once_through_rename_chains() {
    let work = tempfile::tempdir().unwrap();
    let classes = compile(
        work.path(),
        &[
            (
                "Main.java",
                r#"package old;
public class Main {
  public static void run(boolean choose) throws Exception {
    String name = choose ? "old.A" : "old.B";
    System.out.println(name);
    System.out.println(Class.forName(name).getName());
    System.out.println(Class.forName(name, false, Main.class.getClassLoader()).getName());
  }
  public static void main(String[] args) throws Exception { run(true); run(false); }
}"#,
            ),
            ("A.java", "package old; public class A {}"),
            ("B.java", "package old; public class B {}"),
        ],
    );
    let input = Input::read_for_remap(&classes).unwrap();
    let program = Program::from_input(&input, &[]).unwrap();
    let mut overrides = NameOverrides::default();
    overrides
        .classes
        .insert(program.id("old/Main").unwrap(), "newpkg/Main".into());
    overrides
        .classes
        .insert(program.id("old/A").unwrap(), "old/B".into());
    overrides
        .classes
        .insert(program.id("old/B").unwrap(), "newpkg/C".into());
    let names =
        NamePlan::build_with_overrides(&program, &RenameOptions::default(), &overrides).unwrap();
    let output = work.path().join("mapped.jar");
    remap_jar(&output, &program, &names).unwrap();
    assert_eq!(
        common::execute(&output, "newpkg.Main"),
        b"old.A\nold.B\nold.B\nold.B\nnewpkg.C\nnewpkg.C\n"
    );
    let mapped = ClassFile::parse(&jar(&output)["newpkg/Main.class"]).unwrap();
    let original = program.class(program.id("old/Main").unwrap());
    for (original, mapped) in original.methods.iter().zip(&mapped.methods) {
        assert_eq!(
            original.code.as_ref().map(|code| code.bytes.len()),
            mapped.code.as_ref().map(|code| code.bytes.len())
        );
    }
}

#[test]
fn interface_class_name_contract_keeps_receiver_and_original_call_width() {
    let work = tempfile::tempdir().unwrap();
    let classes = compile(
        work.path(),
        &[
            (
                "Main.java",
                r#"package old;
public class Main {
  public static void main(String[] args) throws Exception {
    Lookup receiver = new LookupImpl(); String name = "old.Main";
    System.out.println(name); System.out.println(receiver.lookup(name));
  }
}"#,
            ),
            (
                "Lookup.java",
                "package old; public interface Lookup { String lookup(String name) throws Exception; }",
            ),
            (
                "LookupImpl.java",
                "package old; public class LookupImpl implements Lookup { public String lookup(String name) throws Exception { return Class.forName(name).getName(); } }",
            ),
        ],
    );
    let input = Input::read_for_remap(&classes).unwrap();
    let program = Program::from_input(&input, &[]).unwrap();
    let mut mappings = mappings(
        &program,
        "package old; interface Lookup { String lookup(@ClassName String name) /* was lookup */; }",
    );
    mappings
        .names
        .classes
        .insert(program.id("old/Main").unwrap(), "newpkg/Main".into());
    let names =
        NamePlan::build_with_overrides(&program, &RenameOptions::default(), &mappings.names)
            .unwrap();
    let output = work.path().join("mapped.jar");
    remap_jar_with_mappings(&output, &program, &names, &mappings).unwrap();
    assert_eq!(
        common::execute(&output, "newpkg.Main"),
        b"old.Main\nnewpkg.Main\n"
    );
}

#[test]
fn string_pool_reuse_does_not_follow_rename_chains_twice() {
    let work = tempfile::tempdir().unwrap();
    let classes = compile(
        work.path(),
        &[
            (
                "Main.java",
                "package old; public class Main { public static void main(String[] args) throws Exception { System.out.println(\"old.A\"); System.out.println(Class.forName(\"old.A\").getName()); System.out.println(Class.forName(\"old.B\").getName()); } }",
            ),
            ("A.java", "package old; public class A {}"),
            ("B.java", "package old; public class B {}"),
        ],
    );
    let input = Input::read_for_remap(&classes).unwrap();
    let program = Program::from_input(&input, &[]).unwrap();
    let mut overrides = NameOverrides::default();
    overrides
        .classes
        .insert(program.id("old/Main").unwrap(), "newpkg/Main".into());
    overrides
        .classes
        .insert(program.id("old/A").unwrap(), "old/B".into());
    overrides
        .classes
        .insert(program.id("old/B").unwrap(), "newpkg/C".into());
    let names =
        NamePlan::build_with_overrides(&program, &RenameOptions::default(), &overrides).unwrap();
    let output = work.path().join("mapped.jar");
    remap_jar(&output, &program, &names).unwrap();
    assert_eq!(
        common::execute(&output, "newpkg.Main"),
        b"old.A\nold.B\nnewpkg.C\n"
    );
}

#[test]
fn opaque_attributes_modified_utf8_and_shared_name_roles_survive() {
    fn text(pool: &mut Vec<u8>, bytes: &[u8]) {
        pool.push(1);
        pool.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        pool.extend_from_slice(bytes);
    }
    fn edge(pool: &mut Vec<u8>, tag: u8, index: u16) {
        pool.push(tag);
        pool.extend_from_slice(&index.to_be_bytes());
    }
    let work = tempfile::tempdir().unwrap();
    let mut bytes = b"\xca\xfe\xba\xbe\x00\x03\x00\x2d\x00\x10".to_vec();
    text(&mut bytes, b"old/A");
    edge(&mut bytes, 7, 1);
    text(&mut bytes, b"java/lang/Object");
    edge(&mut bytes, 7, 3);
    text(&mut bytes, b"f");
    text(&mut bytes, b"Lold/A;");
    text(&mut bytes, b"Opaque");
    edge(&mut bytes, 8, 1);
    edge(&mut bytes, 8, 6);
    text(&mut bytes, b"\xc0\x80\xed\xa0\x80");
    edge(&mut bytes, 8, 10);
    text(&mut bytes, b"Signature");
    text(&mut bytes, b"<LongName:Lold/A;>Ljava/lang/Object;");
    text(&mut bytes, b"ext/\xed\xa0\x80");
    edge(&mut bytes, 7, 14);
    for value in [0x21u16, 2, 4, 0, 1, 1, 5, 6, 0, 0, 2] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    let opaque = b"\x00\x07\x00\x00\x00\x06\x00\x01\x00\x08\xff\x00";
    bytes.extend_from_slice(opaque);
    bytes.extend_from_slice(b"\x00\x0c\x00\x00\x00\x02\x00\x0d");
    let source = work.path().join("input.class");
    fs::write(&source, &bytes).unwrap();
    let input = Input::read_for_remap(&source).unwrap();
    let program = Program::from_input(&input, &[]).unwrap();
    let mut overrides = NameOverrides::default();
    overrides.classes.insert(0, "newpkg/Readable".into());
    overrides
        .fields
        .insert(MemberId { class: 0, index: 0 }, "readable".into());
    let names =
        NamePlan::build_with_overrides(&program, &RenameOptions::default(), &overrides).unwrap();
    let output = work.path().join("mapped.jar");
    remap_jar(&output, &program, &names).unwrap();
    let entries = jar(&output);
    let bytes = &entries["newpkg/Readable.class"];
    assert!(bytes.windows(opaque.len()).any(|window| window == opaque));
    assert!(
        bytes
            .windows(8)
            .any(|window| window == b"\x01\x00\x05\xc0\x80\xed\xa0\x80")
    );
    let mapped = ClassFile::parse(bytes).unwrap();
    assert_eq!(mapped.constant_pool.utf8(1).unwrap(), "old/A");
    assert_eq!(
        mapped.constant_pool.string_utf16(8).unwrap(),
        "old/A".encode_utf16().collect::<Vec<_>>()
    );
    assert_eq!(
        mapped.constant_pool.string_utf16(9).unwrap(),
        "Lold/A;".encode_utf16().collect::<Vec<_>>()
    );
    assert_eq!(mapped.constant_pool.string_utf16(11).unwrap(), [0, 0xd800]);
    assert!(matches!(
        mapped.constant_pool.entries[15],
        sporeflower::classfile::Constant::Class(14)
    ));
    assert_eq!(mapped.fields[0].name, "readable");
    assert_eq!(mapped.fields[0].descriptor, "Lnewpkg/Readable;");
    assert!(mapped.constant_pool.entries.iter().any(|constant| matches!(constant, sporeflower::classfile::Constant::Utf8(text) if text == "<LongName:Lnewpkg/Readable;>Ljava/lang/Object;")));
}

#[test]
fn legacy_shared_reflection_bridge_has_cldc_stack_maps() {
    let Some(compiler) = common::legacy_javac() else {
        return;
    };
    let work = tempfile::tempdir().unwrap();
    let modern = compile(
        work.path(),
        &[(
            "Main.java",
            "package old; public class Main { public static void main(String[] args) throws Exception { String name=\"old.Main\"; System.out.println(name); System.out.println(Class.forName(name).getName()); } }",
        )],
    );
    let input = Input::read_for_remap(&modern).unwrap();
    let api = sporeflower::api::ApiCatalog::bundled()
        .unwrap()
        .select(input.classes(), &Default::default())
        .unwrap();
    let api_path = work.path().join("api.jar");
    api.write_jar(&api_path).unwrap();
    let legacy = work.path().join("legacy");
    fs::create_dir_all(&legacy).unwrap();
    common::checked(
        common::legacy_command(compiler, &api_path)
            .arg("-d")
            .arg(&legacy)
            .arg(work.path().join("source/Main.java")),
    );
    let input = Input::read_for_remap(&legacy).unwrap();
    assert_eq!(input.classes()[0].major_version, 45);
    let program = Program::from_input(&input, &[]).unwrap();
    let mut overrides = NameOverrides::default();
    overrides.classes.insert(0, "newpkg/Main".into());
    let names =
        NamePlan::build_with_overrides(&program, &RenameOptions::default(), &overrides).unwrap();
    let output = work.path().join("mapped.jar");
    remap_jar(&output, &program, &names).unwrap();
    assert_eq!(
        common::execute(&output, "newpkg.Main"),
        b"old.Main\nnewpkg.Main\n"
    );
    let entries = jar(&output);
    let bytes = &entries["newpkg/Main.class"];
    let mapped = ClassFile::parse(bytes).unwrap();
    assert_eq!(mapped.major_version, 45);
    assert!(mapped.constant_pool.entries.iter().any(|constant| matches!(constant, sporeflower::classfile::Constant::Utf8(text) if text == "StackMap")));
    assert!(!mapped.constant_pool.entries.iter().any(|constant| matches!(constant, sporeflower::classfile::Constant::Utf8(text) if text == "StackMapTable")));
    let helper = mapped
        .methods
        .iter()
        .find(|method| method.name.starts_with("$sf$className$"))
        .unwrap();
    assert!(helper.synthetic);
    assert_eq!(helper.access_flags & 0x1000, 0);
}

#[test]
fn shared_class_name_fields_translate_at_static_and_initialized_instance_stores() {
    let work = tempfile::tempdir().unwrap();
    let classes = compile(
        work.path(),
        &[(
            "Main.java",
            "package old; public class Main { public static String NAME; public String instance; public Main() { String shared=\"old.Main\"; System.out.println(shared); instance=shared; } public static void main(String[] args) throws Exception { String shared=\"old.Main\"; System.out.println(shared); NAME=shared; System.out.println(Class.forName(\"old.Main\").getName()); System.out.println(Class.forName(NAME).getName()); System.out.println(Class.forName(new Main().instance).getName()); } }",
        )],
    );
    let input = Input::read_for_remap(&classes).unwrap();
    let program = Program::from_input(&input, &[]).unwrap();
    let mut mappings = mappings(
        &program,
        "package old; class Main { @ClassName String NAME /* was NAME */; @ClassName String instance /* was instance */; }",
    );
    mappings.names.classes.insert(0, "newpkg/Main".into());
    let names =
        NamePlan::build_with_overrides(&program, &RenameOptions::default(), &mappings.names)
            .unwrap();
    let output = work.path().join("mapped.jar");
    remap_jar_with_mappings(&output, &program, &names, &mappings).unwrap();
    assert_eq!(
        common::execute(&output, "newpkg.Main"),
        b"old.Main\nnewpkg.Main\nnewpkg.Main\nold.Main\nnewpkg.Main\n"
    );
}

#[test]
fn final_field_adapter_refusal_keeps_previous_artifact() {
    let work = tempfile::tempdir().unwrap();
    let classes = compile(
        work.path(),
        &[(
            "Main.java",
            "package old; public class Main { public static final String NAME; static { String shared=\"old.Main\"; System.out.println(shared); NAME=shared; } }",
        )],
    );
    let input = Input::read_for_remap(&classes).unwrap();
    let program = Program::from_input(&input, &[]).unwrap();
    let mut mappings = mappings(
        &program,
        "package old; class Main { @ClassName String NAME /* was NAME */; }",
    );
    mappings.names.classes.insert(0, "newpkg/Main".into());
    let names =
        NamePlan::build_with_overrides(&program, &RenameOptions::default(), &mappings.names)
            .unwrap();
    let output = work.path().join("mapped.jar");
    fs::write(&output, b"previous artifact").unwrap();
    let error = remap_jar_with_mappings(&output, &program, &names, &mappings).unwrap_err();
    assert!(
        format!("{error:#}").contains("cannot move final field write"),
        "{error:#}"
    );
    assert_eq!(fs::read(output).unwrap(), b"previous artifact");
}

#[test]
fn field_adapter_does_not_pass_uninitialized_this_before_delegation() {
    let work = tempfile::tempdir().unwrap();
    let classes = compile(
        work.path(),
        &[(
            "Main.java",
            "package old; public class Main { public String NAME; public Main() { String shared=\"old.Main\"; System.out.println(shared); NAME=shared; } public static void main(String[] args) { new Main(); } }",
        )],
    );
    let path = classes.join("old/Main.class");
    let mut bytes = fs::read(&path).unwrap();
    let parsed = ClassFile::parse(&bytes).unwrap();
    let code = &parsed
        .methods
        .iter()
        .find(|method| method.name == "<init>")
        .unwrap()
        .code
        .as_ref()
        .unwrap()
        .bytes;
    assert_eq!(&code[..2], &[0x2a, 0xb7]);
    assert_eq!(code.last(), Some(&0xb1));
    let start = bytes
        .windows(code.len())
        .position(|window| window == code)
        .unwrap();
    let mut reordered = code[4..code.len() - 1].to_vec();
    reordered.extend_from_slice(&code[..4]);
    reordered.push(0xb1);
    bytes[start..start + code.len()].copy_from_slice(&reordered);
    fs::write(&path, bytes).unwrap();
    // This is valid authored JVM bytecode: own fields may be assigned before
    // super(), but uninitialized this cannot be passed through a static call.
    assert_eq!(common::execute(&classes, "old.Main"), b"old.Main\n");
    let input = Input::read_for_remap(&classes).unwrap();
    let program = Program::from_input(&input, &[]).unwrap();
    let mut mappings = mappings(
        &program,
        "package old; class Main { @ClassName String NAME /* was NAME */; }",
    );
    mappings.names.classes.insert(0, "newpkg/Main".into());
    let names =
        NamePlan::build_with_overrides(&program, &RenameOptions::default(), &mappings.names)
            .unwrap();
    let error =
        remap_jar_with_mappings(&work.path().join("mapped.jar"), &program, &names, &mappings)
            .unwrap_err();
    assert!(
        format!("{error:#}").contains("uninitialized this"),
        "{error:#}"
    );
}

#[test]
fn inherited_protected_field_adapter_keeps_the_required_receiver_type() {
    use sporeflower::classfile::Constant;
    let work = tempfile::tempdir().unwrap();
    let classes = compile(
        work.path(),
        &[
            (
                "Base.java",
                "package base; public class Base { protected String NAME; }",
            ),
            (
                "Main.java",
                "package old; public class Main extends base.Base { void run() throws Exception { String shared=\"old.Main\"; System.out.println(shared); NAME=shared; System.out.println(Class.forName(NAME).getName()); } public static void main(String[] args) throws Exception { new Main().run(); } }",
            ),
        ],
    );
    let path = classes.join("old/Main.class");
    let mut bytes = fs::read(&path).unwrap();
    let parsed = ClassFile::parse(&bytes).unwrap();
    let base = parsed
        .constant_pool
        .entries
        .iter()
        .enumerate()
        .find_map(|(index, constant)| {
            matches!(constant, Constant::Class(_))
                .then(|| {
                    parsed
                        .constant_pool
                        .class_name(index as u16)
                        .ok()
                        .filter(|name| *name == "base/Base")
                        .map(|_| index as u16)
                })
                .flatten()
        })
        .unwrap();
    for (index, constant) in parsed.constant_pool.entries.iter().enumerate() {
        if let Constant::Fieldref {
            class_index,
            name_and_type_index,
        } = constant
            && parsed.constant_pool.member_ref(index as u16).unwrap().name == "NAME"
        {
            let mut pattern = vec![9];
            pattern.extend_from_slice(&class_index.to_be_bytes());
            pattern.extend_from_slice(&name_and_type_index.to_be_bytes());
            let offset = bytes
                .windows(5)
                .position(|window| window == pattern)
                .unwrap();
            bytes[offset + 1..offset + 3].copy_from_slice(&base.to_be_bytes());
        }
    }
    fs::write(&path, bytes).unwrap();
    assert_eq!(
        common::execute(&classes, "old.Main"),
        b"old.Main\nold.Main\n"
    );
    let input = Input::read_for_remap(&classes).unwrap();
    let program = Program::from_input(&input, &[]).unwrap();
    let mut mappings = mappings(
        &program,
        "package base; class Base { @ClassName String NAME /* was NAME */; }",
    );
    mappings
        .names
        .classes
        .insert(program.id("old/Main").unwrap(), "newpkg/Main".into());
    let names =
        NamePlan::build_with_overrides(&program, &RenameOptions::default(), &mappings.names)
            .unwrap();
    let output = work.path().join("mapped.jar");
    remap_jar_with_mappings(&output, &program, &names, &mappings).unwrap();
    assert_eq!(
        common::execute(&output, "newpkg.Main"),
        b"old.Main\nnewpkg.Main\n"
    );
}

#[test]
fn local_array_reflection_keeps_ordinary_elements_and_maps_only_the_call() {
    let work = tempfile::tempdir().unwrap();
    let classes = compile(
        work.path(),
        &[(
            "Main.java",
            "package old; public class Main { public static void main(String[] args) throws Exception { String[] names=new String[] { \"old.Main\", \"[Lold.Main;\" }; for(int i=0;i<2;i++) { String value=names[i]; System.out.println(value); System.out.println(Class.forName(value).getName()); } System.out.println(names[0]); } }",
        )],
    );
    let input = Input::read_for_remap(&classes).unwrap();
    let program = Program::from_input(&input, &[]).unwrap();
    let mut overrides = NameOverrides::default();
    overrides.classes.insert(0, "newpkg/Main".into());
    let names =
        NamePlan::build_with_overrides(&program, &RenameOptions::default(), &overrides).unwrap();
    let output = work.path().join("mapped.jar");
    remap_jar(&output, &program, &names).unwrap();
    assert_eq!(
        common::execute(&output, "newpkg.Main"),
        b"old.Main\nnewpkg.Main\n[Lold.Main;\n[Lnewpkg.Main;\nold.Main\n"
    );
}

#[test]
fn native_remap_uses_the_loaded_class_after_the_source_changes() {
    fn specimen(text: &str) -> Vec<u8> {
        fn utf(bytes: &mut Vec<u8>, text: &str) {
            bytes.push(1);
            bytes.extend_from_slice(&(text.len() as u16).to_be_bytes());
            bytes.extend_from_slice(text.as_bytes());
        }
        let mut bytes = vec![0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 49, 0, 11];
        utf(&mut bytes, "old/A");
        bytes.extend_from_slice(&[7, 0, 1]);
        utf(&mut bytes, "java/lang/Object");
        bytes.extend_from_slice(&[7, 0, 3]);
        utf(&mut bytes, "NAME");
        utf(&mut bytes, "Ljava/lang/String;");
        utf(&mut bytes, text);
        bytes.extend_from_slice(&[8, 0, 7]);
        utf(&mut bytes, "ConstantValue");
        // Preserve even unused NaN payload bits while remapping.
        bytes.extend_from_slice(&[4, 0x7f, 0xc0, 0, 1]);
        bytes.extend_from_slice(&[
            0, 33, 0, 2, 0, 4, 0, 0, 0, 1, 0, 25, 0, 5, 0, 6, 0, 1, 0, 9, 0, 0, 0, 2, 0, 8, 0, 0,
            0, 0,
        ]);
        bytes
    }
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("A.class");
    let output = work.path().join("mapped.jar");
    let original = specimen("old.A");
    fs::write(&source, &original).unwrap();
    let input = Input::read_for_remap(&source).unwrap();
    let program = Program::from_input(&input, &[]).unwrap();
    let mut mappings = mappings(
        &program,
        "package old; class A { @ClassName String NAME /* was NAME */; }",
    );
    mappings.names.classes.insert(0, "newpkg/A".into());
    let names =
        NamePlan::build_with_overrides(&program, &RenameOptions::default(), &mappings.names)
            .unwrap();
    remap_jar_with_mappings(&output, &program, &names, &mappings).unwrap();
    let previous = fs::read(&output).unwrap();
    fs::write(&source, specimen("other.B")).unwrap();
    remap_jar_with_mappings(&output, &program, &names, &mappings).unwrap();
    assert_eq!(fs::read(&output).unwrap(), previous);
    let mapped = ClassFile::parse(&jar(&output)["newpkg/A.class"]).unwrap();
    assert!(
        matches!(mapped.constant_pool.entries[10], sporeflower::classfile::Constant::Float(value) if value.to_bits() == 0x7fc00001)
    );
    assert_eq!(
        mapped
            .constant_pool
            .string_utf16(mapped.fields[0].constant_value.unwrap())
            .unwrap(),
        "newpkg.A".encode_utf16().collect::<Vec<_>>()
    );
    // Deliberately edited models are detached from native input ownership.
    let mut edited = input.classes().to_vec();
    edited[0].fields[0].access_flags ^= 0x10;
    let edited_program = Program::new(&edited, &[]).unwrap();
    let error = remap_jar(&output, &edited_program, &names).unwrap_err();
    assert!(format!("{error:#}").contains("Input::read_for_remap"));
    assert_eq!(fs::read(&output).unwrap(), previous);
}

#[test]
fn native_remap_retains_archive_and_directory_resources_after_loading() {
    const CLASS: &[u8] = include_bytes!("fixtures/legacy/classes/pkg/TestConsecutiveReads.class");
    for archive in [false, true] {
        let work = tempfile::tempdir().unwrap();
        let source = work
            .path()
            .join(if archive { "input.jar" } else { "classes" });
        let entries = [
            ("unrelated-name.class", CLASS),
            ("pkg/data.bin", &b"original resource"[..]),
            ("icon.class", &b"not bytecode"[..]),
            (
                "META-INF/MANIFEST.MF",
                &b"Manifest-Version: 1.0\r\nMain-Class: pkg.TestConsecutiveReads\r\n\r\n"[..],
            ),
        ];
        if archive {
            let mut jar = zip::ZipWriter::new(fs::File::create(&source).unwrap());
            for (name, bytes) in entries {
                jar.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                jar.write_all(bytes).unwrap();
            }
            jar.finish().unwrap();
        } else {
            for (name, bytes) in entries {
                let path = source.join(name);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, bytes).unwrap();
            }
        }
        let input = Input::read_for_remap(&source).unwrap();
        fs::rename(&source, work.path().join("moved-input")).unwrap();
        let program = Program::from_input(&input, &[]).unwrap();
        let mut overrides = NameOverrides::default();
        overrides.classes.insert(0, "newpkg/Readable".into());
        let names = NamePlan::build_with_overrides(&program, &RenameOptions::default(), &overrides)
            .unwrap();
        let destination = work.path().join("mapped.jar");
        remap_jar(&destination, &program, &names).unwrap();
        let mapped = jar(&destination);
        assert_eq!(
            ClassFile::parse(&mapped["newpkg/Readable.class"])
                .unwrap()
                .name,
            "newpkg/Readable"
        );
        assert_eq!(mapped["pkg/data.bin"], b"original resource");
        assert_eq!(mapped["newpkg/data.bin"], b"original resource");
        assert_eq!(mapped["icon.class"], b"not bytecode");
        assert!(
            String::from_utf8_lossy(&mapped["META-INF/MANIFEST.MF"])
                .contains("Main-Class: newpkg.Readable")
        );
    }
}

#[test]
fn overflow_preserves_connected_call_and_field_origins_through_rename_chains() {
    let work = tempfile::tempdir().unwrap();
    let mut source = String::from(
        "package old; public class Main { static String saved; public static void main(String[] args) throws Exception { String first = \"old.A\"; String second = \"old.B\"; String third = \"old.C\"; System.out.println(Class.forName(args.length == 0 ? first : second).getName()); saved = args.length == 0 ? second : third; System.out.println(saved); System.out.println(Class.forName(third).getName()); System.out.println(\"old.A\"); } ",
    );
    for index in 0..150 {
        write!(
            source,
            "public static final String CONSTANT_{index}=\"unused {index}\";"
        )
        .unwrap();
    }
    source.push('}');
    let classes = compile(
        work.path(),
        &[
            ("Main.java", &source),
            ("A.java", "package old; public class A {}"),
            ("B.java", "package old; public class B {}"),
            ("C.java", "package old; public class C {}"),
        ],
    );
    let input = Input::read_for_remap(&classes).unwrap();
    let program = Program::from_input(&input, &[]).unwrap();
    let mut mappings = mappings(
        &program,
        "package old; class Main { @ClassName String saved /* was saved */; }",
    );
    for (original, mapped) in [
        ("old/Main", "newpkg/Main"),
        ("old/A", "old/B"),
        ("old/B", "old/C"),
        ("old/C", "newpkg/D"),
    ] {
        mappings
            .names
            .classes
            .insert(program.id(original).unwrap(), mapped.into());
    }
    assert!(
        program
            .class(program.id("old/Main").unwrap())
            .constant_pool
            .entries
            .len()
            > 255
    );
    let names =
        NamePlan::build_with_overrides(&program, &RenameOptions::default(), &mappings.names)
            .unwrap();
    let output = work.path().join("mapped.jar");
    remap_jar_with_mappings(&output, &program, &names, &mappings).unwrap();
    assert_eq!(
        common::execute(&output, "newpkg.Main"),
        b"old.B\nold.C\nnewpkg.D\nold.A\n"
    );
}

#[test]
fn overflow_rejects_a_connected_opaque_consumer_without_publishing() {
    let work = tempfile::tempdir().unwrap();
    let mut source = String::from(
        "package old; public class Main { public static void main(String[] args) throws Exception { String name = \"old.A\"; System.out.println(Class.forName(name).getName()); System.out.println(Class.forName(args.length == 0 ? name : args[0]).getName()); System.out.println(\"old.A\"); } ",
    );
    for index in 0..150 {
        write!(
            source,
            "public static final String CONSTANT_{index}=\"unused {index}\";"
        )
        .unwrap();
    }
    source.push('}');
    let classes = compile(
        work.path(),
        &[
            ("Main.java", &source),
            ("A.java", "package old; public class A {}"),
        ],
    );
    let input = Input::read_for_remap(&classes).unwrap();
    let program = Program::from_input(&input, &[]).unwrap();
    let mut overrides = NameOverrides::default();
    overrides
        .classes
        .insert(program.id("old/A").unwrap(), "newpkg/A".into());
    let names =
        NamePlan::build_with_overrides(&program, &RenameOptions::default(), &overrides).unwrap();
    let output = work.path().join("mapped.jar");
    fs::write(&output, b"previous generation").unwrap();
    let error = remap_jar(&output, &program, &names).unwrap_err();
    let error = format!("{error:#}");
    assert!(
        error.contains("unsupported shared ClassName consumer"),
        "{error}"
    );
    assert!(error.contains("opaque input"), "{error}");
    assert_eq!(fs::read(&output).unwrap(), b"previous generation");
}
