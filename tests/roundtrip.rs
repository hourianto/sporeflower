//! End-to-end semantic checks use isolated original/recompiled class directories.
use crate::common::{checked, compile, decompile, execute, fixture_roundtrip, sources};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[test]
fn parallel_phi_expressions_read_the_old_values() {
    fixture_roundtrip("PhiProbe");
}

#[test]
fn array_initializer_captures_keep_retained_index_calculations_after_their_definitions() {
    fixture_roundtrip("IndexedInitializer");
}

#[test]
fn coalesced_storage_keeps_exception_snapshots_captures_and_loop_iterations() {
    fixture_roundtrip("CoalescingProbe");
}

#[test]
fn class_initialization_precedes_constructor_argument_effects() {
    fixture_roundtrip("AllocationOrderProbe");
}

#[test]
fn array_elements_follow_final_phi_types() {
    fixture_roundtrip("ArrayPhiProbe");
}

#[test]
fn interface_initializers_preserve_partial_field_observations() {
    fixture_roundtrip("InterfaceOrder");
}

#[test]
fn constructor_arguments_preserve_captured_values() {
    fixture_roundtrip("ConstructorCaptures");
}

#[test]
fn real_jsr_finally_subroutines_preserve_continuations() {
    let work = tempfile::tempdir().unwrap();
    let original = Path::new("tests/fixtures/legacy-jsr/classes");
    let output = work.path().join("source");
    let recompiled = work.path().join("recompiled");
    let expected = execute(original, "LegacyFinally");
    decompile(original, &output);
    compile(&sources(&output), &recompiled, None);
    assert_eq!(expected, execute(&recompiled, "LegacyFinally"));
}

#[test]
fn java_control_flow_roundtrip_preserves_behavior() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let output = work.path().join("source");
    let recompiled = work.path().join("recompiled");
    compile(
        &[PathBuf::from("tests/fixtures/java/Flow.java")],
        &original,
        None,
    );
    let expected = execute(&original, "Flow");
    decompile(&original, &output);
    compile(&sources(&output), &recompiled, None);
    assert_eq!(expected, execute(&recompiled, "Flow"));
}

#[test]
fn legacy_stack_and_allocation_roundtrip_preserves_behavior() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let output = work.path().join("source");
    let recompiled = work.path().join("recompiled");
    fs::create_dir_all(original.join("pkg")).unwrap();
    for entry in fs::read_dir("tests/fixtures/legacy/classes/pkg").unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), original.join("pkg").join(entry.file_name())).unwrap();
    }
    let driver = PathBuf::from("tests/fixtures/java/LegacyDriver.java");
    compile(std::slice::from_ref(&driver), &original, Some(&original));
    let expected = execute(&original, "LegacyDriver");
    decompile(Path::new("tests/fixtures/legacy/classes"), &output);
    let mut files = sources(&output);
    files.push(driver);
    compile(&files, &recompiled, None);
    assert_eq!(expected, execute(&recompiled, "LegacyDriver"));
}

#[test]
fn source_binding_roundtrip_preserves_fields_and_overloads() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let output = work.path().join("source");
    let recompiled = work.path().join("recompiled");
    compile(
        &[PathBuf::from("tests/fixtures/java/Binding.java")],
        &original,
        None,
    );
    let expected = execute(&original, "Binding");
    decompile(&original, &output);
    compile(&sources(&output), &recompiled, None);
    assert_eq!(expected, execute(&recompiled, "Binding"));
}

#[test]
fn different_field_descriptors_do_not_capture_inherited_source_binding() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &sources(Path::new("tests/fixtures/field_binding")),
        &original,
        None,
    );
    let path = original.join("hidden/PublicChild.class");
    let bytes = replace_class_utf8(&fs::read(&path).unwrap(), |text| {
        if text == "mark" {
            "link".into()
        } else {
            text.into()
        }
    });
    fs::write(path, bytes).unwrap();
    // The client refers to Object link through the public child. JVM lookup
    // passes the child's String link and reaches its inaccessible base class.
    let expected = execute(&original, "client.FieldBinding");
    assert_eq!(expected, b"true\ntrue\nchild\n");
    decompile(&original, &source);
    compile(&sources(&source), &rebuilt, None);
    assert_eq!(expected, execute(&rebuilt, "client.FieldBinding"));
}

#[test]
fn final_field_defaults_belong_to_each_initializing_constructor() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from("tests/fixtures/java/FinalDefaults.java")],
        &original,
        None,
    );
    let path = original.join("FinalDefaults.class");
    let mut bytes = fs::read(&path).unwrap();
    let class = sporeflower::classfile::ClassFile::parse(&bytes).unwrap();
    let code = &class
        .methods
        .iter()
        .find(|method| method.name == "<init>" && method.descriptor == "()V")
        .unwrap()
        .code
        .as_ref()
        .unwrap()
        .bytes;
    let starts: Vec<_> = bytes
        .windows(code.len())
        .enumerate()
        .filter_map(|(at, value)| (value == code).then_some(at))
        .collect();
    assert_eq!(starts.len(), 1);
    let mut stores = 0;
    for instruction in sporeflower::bytecode::decode(code).unwrap() {
        if instruction.opcode == 0xb5 {
            let at = instruction.pc as usize;
            assert_eq!(code[at - 2], 0x2a); // aload_0
            assert!(matches!(code[at - 1], 0x01 | 0x03)); // null or zero
            bytes[starts[0] + at - 2..starts[0] + at + 3].fill(0x00);
            stores += 1;
        }
    }
    assert_eq!(stores, 2);
    fs::write(path, bytes).unwrap();
    let expected = execute(&original, "FinalDefaults");
    assert_eq!(expected, b"0:null\n7:written\n11:written\n");
    decompile(&original, &source);
    compile(&sources(&source), &rebuilt, None);
    assert_eq!(expected, execute(&rebuilt, "FinalDefaults"));
    let class = sporeflower::classfile::ClassFile::parse(
        &fs::read(rebuilt.join("FinalDefaults.class")).unwrap(),
    )
    .unwrap();
    assert!(
        class
            .fields
            .iter()
            .all(|field| field.access_flags & 0x10 != 0 && field.constant_value.is_none())
    );
}

#[test]
fn bytecode_reference_operations_preserve_nulls_and_cast_failures() {
    fixture_roundtrip("ReferenceOperations");
}

#[test]
fn conditional_coercions_keep_java_expression_precedence() {
    fixture_roundtrip("ConditionalPrecedence");
}

#[test]
fn literal_cfg_edges_preserve_effects_and_remove_dead_throws_and_returns() {
    fixture_roundtrip("LiteralBranches");
}

#[test]
fn sparse_array_initializers_preserve_defaults_order_escape_and_failure_state() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from("tests/fixtures/java/SparseArrays.java")],
        &original,
        None,
    );
    let expected = execute(&original, "SparseArrays");
    decompile(&original, &source);
    let text = fs::read_to_string(source.join("SparseArrays.java")).unwrap();
    let method = |name| crate::common::source::method(&text, name);
    for name in ["sparse", "defaults"] {
        let body = method(name);
        assert!(body.contains("new byte[300]"), "{body}");
        assert!(!body.contains("new byte[]"), "{body}");
    }
    assert!(!method("defaults").contains("] ="), "{text}");
    assert!(method("dense").contains("new int[]"), "{text}");
    assert!(method("nested").contains("return new int[][]"), "{text}");
    assert!(!method("nested").contains("] ="), "{text}");
    assert!(method("compactRows").contains("new int[][]"), "{text}");
    assert!(!method("compactRows").contains("] ="), "{text}");
    assert!(method("nulls").contains("new Object[3]"), "{text}");
    assert!(method("zeroFloats").contains("new float[3]"), "{text}");
    assert_eq!(
        method("effectfulZeros").matches("mark(0)").count(),
        2,
        "{text}"
    );
    compile(&sources(&source), &rebuilt, None);
    assert_eq!(expected, execute(&rebuilt, "SparseArrays"));
}

#[test]
fn early_loop_exits_preserve_terminal_tail_effects_without_cloning() {
    fixture_roundtrip("EarlyLoopExits");
}

#[test]
fn typed_string_concatenation_preserves_overloads_order_and_identity() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let output = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from(
            "tests/fixtures/java/StringConcatenation.java",
        )],
        &original,
        None,
    );
    let expected = execute(&original, "StringConcatenation");
    decompile(&original, &output);
    let source = fs::read_to_string(output.join("StringConcatenation.java")).unwrap();
    assert!(source.contains("\"symbol \" +"), "{source}");
    assert!(
        source.contains("(int)"),
        "numeric character append must stay numeric: {source}"
    );
    assert!(
        source.contains("\"\" +"),
        "numeric/Object prefixes must keep conversion boundaries: {source}"
    );
    let object_first = source
        .split("static String objectFirst")
        .nth(1)
        .unwrap()
        .split("static void discarded")
        .next()
        .unwrap();
    assert!(
        object_first.contains("return \"\" +"),
        "Object conversion order must be exercised by a recovered concat: {object_first}"
    );
    assert!(
        source.contains("new String(\"a\" + \"bc\")"),
        "constant-only chains must retain fresh identity: {source}"
    );
    for name in [
        "wrappedObject",
        "nestedWrapped",
        "wrappedSaved",
        "wrappedNumbers",
        "wrappedConstant",
        "builderDirect",
        "builderWrapped",
        "wrappedBuilderString",
    ] {
        let body = crate::common::source::method(&source, name);
        assert!(body.contains(" + "), "{name} must recover concat: {body}");
        assert!(
            !body.contains("StringBuffer")
                && !body.contains("StringBuilder")
                && !body.contains("String.valueOf"),
            "{name} must consume the fresh builder and terminal conversion: {body}"
        );
    }
    let saved = crate::common::source::method(&source, "wrappedSaved");
    assert_eq!(saved.matches("next()").count(), 1, "{saved}");
    assert!(
        saved.contains(" = "),
        "shared conversion needs storage: {saved}"
    );
    assert!(
        crate::common::source::method(&source, "wrappedConstant")
            .contains("new String(\"a\" + \"bc\")"),
        "wrapped constant chains must retain fresh identity: {source}"
    );
    for name in [
        "wrappedCharacters",
        "wrappedSeed",
        "wrappedCapacity",
        "wrappedShared",
        "wrappedEscaped",
    ] {
        let body = crate::common::source::method(&source, name);
        assert!(
            body.contains("StringBuffer"),
            "{name} must retain observable builder behavior: {body}"
        );
    }
    assert!(
        source.contains(".indexOf('/')"),
        "String character-code argument should be readable: {source}"
    );
    assert!(
        source.contains(".lastIndexOf(':', 47)"),
        "character and offset arguments have different roles: {source}"
    );
    compile(&sources(&output), &rebuilt, None);
    assert_eq!(expected, execute(&rebuilt, "StringConcatenation"));
    if let Some(legacy) = crate::common::legacy_javac() {
        let classes: Vec<_> = fs::read_dir(&original)
            .unwrap()
            .map(Result::unwrap)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "class")
            })
            .map(|path| sporeflower::classfile::ClassFile::parse(&fs::read(path).unwrap()).unwrap())
            .collect();
        let api = sporeflower::api::ApiCatalog::bundled()
            .unwrap()
            .select(&classes, &Default::default())
            .unwrap();
        let api_jar = work.path().join("api.jar");
        api.write_jar(&api_jar).unwrap();
        let legacy_classes = work.path().join("legacy");
        fs::create_dir_all(&legacy_classes).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api_jar)
                .arg("-d")
                .arg(&legacy_classes)
                .args(sources(&output)),
        );
        assert_eq!(expected, execute(&legacy_classes, "StringConcatenation"));
    }
}

#[test]
fn field_and_array_updates_preserve_typed_locations_and_saved_operands() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let output = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from("tests/fixtures/java/FieldArrayUpdates.java")],
        &original,
        None,
    );
    let expected = execute(&original, "FieldArrayUpdates");
    decompile(&original, &output);
    let source = fs::read_to_string(output.join("FieldArrayUpdates.java")).unwrap();
    assert!(
        source.contains("this.i +="),
        "field update was not recovered: {source}"
    );
    assert!(
        source.contains("[1] |="),
        "narrow array update was not recovered: {source}"
    );
    assert!(
        source.contains("this.i += 1.0F"),
        "float arithmetic must not become integer increment: {source}"
    );
    assert!(
        source.contains("this.l += 1.0D"),
        "double arithmetic must not become long increment: {source}"
    );
    let saved_field = source
        .split("void savedField()")
        .nth(1)
        .unwrap()
        .split("static void separateReceivers")
        .next()
        .unwrap();
    assert!(
        !saved_field.contains("this.i +="),
        "saved old read must not become a new field read: {saved_field}"
    );
    compile(&sources(&output), &rebuilt, None);
    assert_eq!(expected, execute(&rebuilt, "FieldArrayUpdates"));
    for directory in [&original, &rebuilt] {
        let class = sporeflower::classfile::ClassFile::parse(
            &fs::read(directory.join("FieldArrayUpdates.class")).unwrap(),
        )
        .unwrap();
        assert!(
            class
                .fields
                .iter()
                .find(|field| field.name == "observed")
                .unwrap()
                .access_flags
                & 0x0040
                != 0
        );
        let method = class
            .methods
            .iter()
            .find(|method| method.name == "volatileMutation")
            .unwrap();
        let instructions =
            sporeflower::bytecode::decode(&method.code.as_ref().unwrap().bytes).unwrap();
        assert_eq!(
            instructions
                .iter()
                .filter(|instruction| instruction.opcode == 0xb4)
                .count(),
            1
        );
        assert_eq!(
            instructions
                .iter()
                .filter(|instruction| instruction.opcode == 0xb5)
                .count(),
            1
        );
    }
    if let Some(legacy) = crate::common::legacy_javac() {
        let class = sporeflower::classfile::ClassFile::parse(
            &fs::read(original.join("FieldArrayUpdates.class")).unwrap(),
        )
        .unwrap();
        let api = sporeflower::api::ApiCatalog::bundled()
            .unwrap()
            .select(&[class], &Default::default())
            .unwrap();
        let api_jar = work.path().join("api.jar");
        api.write_jar(&api_jar).unwrap();
        let legacy_classes = work.path().join("legacy");
        fs::create_dir_all(&legacy_classes).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api_jar)
                .arg("-d")
                .arg(&legacy_classes)
                .args(sources(&output)),
        );
        assert_eq!(expected, execute(&legacy_classes, "FieldArrayUpdates"));
    }
}

#[test]
fn protected_field_qualifiers_keep_the_legal_subclass_view() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let output = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &sources(Path::new("tests/fixtures/protected_fields")),
        &original,
        None,
    );
    let expected = execute(&original, "client.ProtectedFields");
    assert_eq!(expected, b"23\n23\n40\n7\n");
    decompile(&original, &output);
    compile(&sources(&output), &rebuilt, None);
    assert_eq!(expected, execute(&rebuilt, "client.ProtectedFields"));
}

#[test]
fn source_updates_keep_saved_operands_and_exact_loop_tail() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from("tests/fixtures/java/UpdateForms.java")],
        &original,
        None,
    );
    let expected = execute(&original, "UpdateForms");
    decompile(&original, &source);
    let text = fs::read_to_string(source.join("UpdateForms.java")).unwrap();
    assert!(text.matches("for (").count() >= 2, "{text}");
    assert!(text.contains("for (int "), "{text}");
    assert!(text.contains("++"), "{text}");
    let counted = text
        .split(" counted(")
        .nth(1)
        .unwrap()
        .split("\n    }")
        .next()
        .unwrap();
    assert!(
        counted.contains("for (") && counted.contains("if (") && !counted.contains("continue;"),
        "{text}"
    );
    compile(&sources(&source), &rebuilt, None);
    assert_eq!(expected, execute(&rebuilt, "UpdateForms"));
    if let Some(legacy) = crate::common::legacy_javac() {
        let class = sporeflower::classfile::ClassFile::parse(
            &fs::read(original.join("UpdateForms.class")).unwrap(),
        )
        .unwrap();
        let api = sporeflower::api::ApiCatalog::bundled()
            .unwrap()
            .select(&[class], &Default::default())
            .unwrap();
        let api_jar = work.path().join("api.jar");
        api.write_jar(&api_jar).unwrap();
        let legacy_classes = work.path().join("legacy");
        fs::create_dir_all(&legacy_classes).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api_jar)
                .arg("-d")
                .arg(&legacy_classes)
                .args(sources(&source)),
        );
        assert_eq!(expected, execute(&legacy_classes, "UpdateForms"));
    }
}

#[test]
fn missing_dispatch_contracts_preserve_linkage_failure_and_override_targets() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let output = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from("tests/fixtures/java/DispatchContracts.java")],
        &original,
        None,
    );
    let path = original.join("DispatchImpl.class");
    let bytes = fs::read(&path).unwrap();
    let bytes = edit_method_table(&bytes, |name, flags| match name {
        "missing" | "gap" => None,
        "narrowed" => Some((flags & !1) | 4),
        _ => Some(flags),
    });
    fs::write(path, bytes).unwrap();
    for class in ["Constructorless", "ConstructorlessChild"] {
        let path = original.join(format!("{class}.class"));
        let bytes = edit_method_table(&fs::read(&path).unwrap(), |name, flags| {
            (name != "<init>").then_some(flags)
        });
        fs::write(path, bytes).unwrap();
    }
    let expected = execute(&original, "DispatchContracts");
    assert_eq!(expected, b"8\n9\ntrue\ntrue\ntrue\ntrue\n0\n");
    decompile(&original, &output);
    let implementation = fs::read_to_string(output.join("DispatchImpl.java")).unwrap();
    assert!(implementation.contains("public int narrowed()"));
    assert_eq!(
        implementation
            .matches("new java.lang.AbstractMethodError()")
            .count(),
        2
    );
    assert!(
        !fs::read_to_string(output.join("DispatchLeaf.java"))
            .unwrap()
            .contains("AbstractMethodError")
    );
    compile(&sources(&output), &rebuilt, None);
    assert_eq!(expected, execute(&rebuilt, "DispatchContracts"));

    // The actual CLDC API omits AbstractMethodError. Its specified Error
    // superclass projection must compile without inventing a platform class.
    let classes: Vec<_> = fs::read_dir(&original)
        .unwrap()
        .map(Result::unwrap)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "class")
                && path.file_stem().unwrap() != "DispatchContracts"
        })
        .map(|path| sporeflower::classfile::ClassFile::parse(&fs::read(path).unwrap()).unwrap())
        .collect();
    let api = sporeflower::api::ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &Default::default())
        .unwrap();
    assert!(
        !api.classes
            .iter()
            .any(|class| class.name == "java/lang/AbstractMethodError")
    );
    let program = sporeflower::program::Program::new(&classes, &api.classes).unwrap();
    let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
    let cldc_source = work.path().join("cldc-source");
    fs::create_dir_all(&cldc_source).unwrap();
    for id in 0..classes.len() {
        let source =
            sporeflower::java::decompile_class(&program, &names, id, &Default::default()).unwrap();
        assert_eq!(source.failed_methods, 0, "{:?}", source.diagnostics);
        assert!(
            !source
                .source
                .contains("new java.lang.AbstractMethodError()")
        );
        fs::write(
            cldc_source.join(format!("{}.java", names.class_name(id))),
            source.source,
        )
        .unwrap();
    }
    let api_jar = work.path().join("cldc-api.jar");
    api.write_jar(&api_jar).unwrap();
    let cldc_classes = work.path().join("cldc-classes");
    fs::create_dir_all(&cldc_classes).unwrap();
    if let Some(legacy) = crate::common::legacy_javac() {
        checked(
            crate::common::legacy_command(legacy, &api_jar)
                .arg("-d")
                .arg(&cldc_classes)
                .args(sources(&cldc_source)),
        );
    } else {
        checked(
            crate::common::compile_command()
                .args(["-bootclasspath"])
                .arg(&api_jar)
                .arg("-classpath")
                .arg(&api_jar)
                .arg("-d")
                .arg(&cldc_classes)
                .args(sources(&cldc_source)),
        );
    }
}

fn edit_method_table(bytes: &[u8], edit: impl Fn(&str, u16) -> Option<u16>) -> Vec<u8> {
    fn u16_at(bytes: &[u8], offset: usize) -> usize {
        u16::from_be_bytes([bytes[offset], bytes[offset + 1]]) as usize
    }
    fn skip_member(bytes: &[u8], mut offset: usize) -> usize {
        let attributes = u16_at(bytes, offset + 6);
        offset += 8;
        for _ in 0..attributes {
            let length =
                u32::from_be_bytes(bytes[offset + 2..offset + 6].try_into().unwrap()) as usize;
            offset += 6 + length;
        }
        offset
    }
    let count = u16_at(bytes, 8);
    let mut strings = vec![""; count];
    let mut offset = 10;
    let mut index = 1;
    while index < count {
        let tag = bytes[offset];
        offset += 1;
        offset += match tag {
            1 => {
                let length = u16_at(bytes, offset);
                strings[index] =
                    std::str::from_utf8(&bytes[offset + 2..offset + 2 + length]).unwrap();
                2 + length
            }
            3 | 4 | 9 | 10 | 11 | 12 | 17 | 18 => 4,
            5 | 6 => {
                index += 1;
                8
            }
            7 | 8 | 16 | 19 | 20 => 2,
            15 => 3,
            _ => panic!("constant tag {tag}"),
        };
        index += 1;
    }
    offset += 6;
    offset += 2 + 2 * u16_at(bytes, offset);
    let fields = u16_at(bytes, offset);
    offset += 2;
    for _ in 0..fields {
        offset = skip_member(bytes, offset);
    }
    let table = offset;
    let methods = u16_at(bytes, offset);
    offset += 2;
    let mut result = bytes[..offset].to_vec();
    let mut retained = 0u16;
    for _ in 0..methods {
        let end = skip_member(bytes, offset);
        let name = strings[u16_at(bytes, offset + 2)];
        if let Some(flags) = edit(name, u16_at(bytes, offset) as u16) {
            result.extend_from_slice(&flags.to_be_bytes());
            result.extend_from_slice(&bytes[offset + 2..end]);
            retained += 1;
        }
        offset = end;
    }
    result[table..table + 2].copy_from_slice(&retained.to_be_bytes());
    result.extend_from_slice(&bytes[offset..]);
    result
}

#[test]
fn synthetic_member_spellings_are_renamed_as_one_identity() {
    fixture_roundtrip("SyntheticMembers");
}

#[test]
fn package_roots_remain_resolvable_when_a_visible_class_shadows_them() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("input");
    let compiled = work.path().join("compiled");
    let original = work.path().join("original");
    let output = work.path().join("source");
    let recompiled = work.path().join("recompiled");
    for package in ["a/left", "a/right", "a/scope"] {
        fs::create_dir_all(input.join(package)).unwrap();
    }
    fs::write(
        input.join("a/left/Item.java"),
        "package a.left; public class Item { public int value() { return 7; } }",
    )
    .unwrap();
    fs::write(
        input.join("a/right/Item.java"),
        "package a.right; public class Item { public int value() { return 11; } }",
    )
    .unwrap();
    fs::write(input.join("a/scope/Shadow.java"), "package a.scope; public class Shadow { public static int sum(a.left.Item x, a.right.Item y) { return x.value() + y.value(); } }").unwrap();
    fs::write(input.join("a/scope/Driver.java"), "package a.scope; public class Driver { public static void main(String[] args) { System.out.println(Shadow.sum(new a.left.Item(), new a.right.Item())); } }").unwrap();
    compile(&sources(&input), &compiled, None);
    for relative in [
        "a/left/Item",
        "a/right/Item",
        "a/scope/Shadow",
        "a/scope/Driver",
    ] {
        let bytes = replace_class_utf8(
            &fs::read(compiled.join(format!("{relative}.class"))).unwrap(),
            |text| text.replace("a/scope/Shadow", "a/scope/a"),
        );
        let class = sporeflower::classfile::ClassFile::parse(&bytes).unwrap();
        let path = original.join(format!("{}.class", class.name));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    let expected = execute(&original, "a.scope.Driver");
    decompile(&original, &output);
    compile(&sources(&output), &recompiled, None);
    assert_eq!(expected, execute(&recompiled, "a.scope.Driver"));
}

#[test]
fn special_dispatch_roundtrip_preserves_exact_nonvirtual_target() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let output = work.path().join("source");
    let recompiled = work.path().join("recompiled");
    compile(
        &[PathBuf::from("tests/fixtures/java/SpecialDispatch.java")],
        &original,
        None,
    );
    let path = original.join("SpecialDispatch.class");
    let mut bytes = fs::read(&path).unwrap();
    let class = sporeflower::classfile::ClassFile::parse(&bytes).unwrap();
    for method in class
        .methods
        .iter()
        .filter(|method| method.name.starts_with("special"))
    {
        let code = &method.code.as_ref().unwrap().bytes;
        let code_offset = bytes
            .windows(code.len())
            .position(|window| window == code)
            .unwrap();
        let instruction = sporeflower::bytecode::decode(code)
            .unwrap()
            .into_iter()
            .find(|instruction| instruction.opcode == 0xb6)
            .unwrap();
        bytes[code_offset + instruction.pc as usize] = 0xb7;
    }
    fs::write(&path, bytes).unwrap();
    let expected = execute(&original, "SpecialDispatch");
    assert_eq!(expected, b"11\n102\n2\n7\n107\n");
    decompile(&original, &output);
    compile(&sources(&output), &recompiled, None);
    assert_eq!(expected, execute(&recompiled, "SpecialDispatch"));
}

#[test]
fn package_relocation_preserves_reflection_and_dynamic_resource_lookup() {
    let work = tempfile::tempdir().unwrap();
    let inputs = work.path().join("input");
    let compiled = work.path().join("compiled");
    let original = work.path().join("original");
    let output = work.path().join("source");
    let recompiled = work.path().join("recompiled");
    fs::create_dir_all(inputs.join("orig")).unwrap();
    fs::create_dir_all(inputs.join("client")).unwrap();
    fs::write(
        inputs.join("orig/DefaultType.java"),
        r#"package orig;
public class DefaultType {
    public static int read(Class type, String name) throws Exception {
        java.io.InputStream stream = type.getResourceAsStream(name);
        try { return stream.read(); } finally { stream.close(); }
    }
}"#,
    )
    .unwrap();
    fs::write(inputs.join("client/NamedUser.java"), r#"package client;
import orig.DefaultType;
public class NamedUser {
    static int effects;
    static Class receiver(boolean nil) { effects = effects * 10 + 1; return nil ? null : DefaultType.class; }
    static String path() { effects = effects * 10 + 2; return "token.txt"; }
    public static void main(String[] args) throws Exception {
        System.out.println(DefaultType.read(receiver(false), path()));
        System.out.println(DefaultType.read(Class.forName("orig.DefaultType"), "token.txt"));
        System.out.println(DefaultType.read(DefaultType[].class, "token.txt"));
        System.out.println(DefaultType.read(NamedUser.class, "token.txt"));
        System.out.println(DefaultType.read(NamedUser.class, "/token.txt"));
        try { DefaultType.read(receiver(true), path()); } catch (NullPointerException expected) {}
        System.out.println(effects);
    }
}"#).unwrap();
    compile(&sources(&inputs), &compiled, None);
    fs::create_dir_all(original.join("client")).unwrap();
    for (input, output) in [
        ("orig/DefaultType.class", "DefaultType.class"),
        ("client/NamedUser.class", "client/NamedUser.class"),
    ] {
        let bytes = fs::read(compiled.join(input)).unwrap();
        fs::write(
            original.join(output),
            replace_class_utf8(&bytes, |text| {
                text.replace("orig/DefaultType", "DefaultType")
                    .replace("orig.DefaultType", "DefaultType")
            }),
        )
        .unwrap();
    }
    fs::write(original.join("token.txt"), [17]).unwrap();
    fs::write(original.join("client/token.txt"), [23]).unwrap();
    let expected = execute(&original, "client.NamedUser");
    assert_eq!(expected, b"17\n17\n17\n23\n17\n1212\n");
    decompile(&original, &output);
    compile(&sources(&output), &recompiled, None);
    fs::write(recompiled.join("token.txt"), [17]).unwrap();
    fs::write(recompiled.join("client/token.txt"), [23]).unwrap();
    assert_eq!(expected, execute(&recompiled, "client.NamedUser"));
}

fn replace_class_utf8(bytes: &[u8], replace: impl Fn(&str) -> String) -> Vec<u8> {
    let count = u16::from_be_bytes([bytes[8], bytes[9]]) as usize;
    let mut output = bytes[..10].to_vec();
    let mut position = 10;
    let mut index = 1;
    while index < count {
        let tag = bytes[position];
        position += 1;
        output.push(tag);
        if tag == 1 {
            let length = u16::from_be_bytes([bytes[position], bytes[position + 1]]) as usize;
            position += 2;
            let text = replace(std::str::from_utf8(&bytes[position..position + length]).unwrap());
            output.extend_from_slice(&(text.len() as u16).to_be_bytes());
            output.extend_from_slice(text.as_bytes());
            position += length;
        } else {
            let length = match tag {
                3 | 4 | 9 | 10 | 11 | 12 | 17 | 18 => 4,
                5 | 6 => {
                    index += 1;
                    8
                }
                7 | 8 | 16 | 19 | 20 => 2,
                15 => 3,
                _ => panic!("constant tag {tag}"),
            };
            output.extend_from_slice(&bytes[position..position + length]);
            position += length;
        }
        index += 1;
    }
    output.extend_from_slice(&bytes[position..]);
    output
}
