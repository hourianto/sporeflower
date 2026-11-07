use sporeflower::classfile::{ClassFile, ConstantPool, Field, Method};
use sporeflower::{
    naming::{NameOverrides, NamePlan, RenameOptions},
    program::{MemberId, Program},
};

fn class(
    name: &str,
    parent: Option<&str>,
    interfaces: &[&str],
    flags: u16,
    methods: &[(&str, &str, u16)],
) -> ClassFile {
    ClassFile {
        minor_version: 0,
        major_version: 47,
        access_flags: flags,
        name: name.into(),
        super_name: parent.map(Into::into),
        interfaces: interfaces.iter().map(|name| (*name).into()).collect(),
        constant_pool: ConstantPool::default(),
        fields: Vec::new(),
        methods: methods
            .iter()
            .map(|(name, descriptor, flags)| Method {
                access_flags: *flags,
                name: (*name).into(),
                descriptor: (*descriptor).into(),
                code: None,
                exceptions: Vec::new(),
                synthetic: false,
            })
            .collect(),
        source_file: None,
        inner_classes: Vec::new(),
        warnings: Vec::new(),
    }
}

#[test]
fn sibling_interface_contracts_share_one_repaired_name() {
    let classes = vec![
        class("First", None, &[], 0x601, &[("if", "()I", 0x401)]),
        class("Second", None, &[], 0x601, &[("if", "()I", 0x401)]),
        class("Both", None, &["First", "Second"], 0x601, &[]),
        class(
            "Impl",
            None,
            &["Both"],
            1,
            &[("if", "()I", 1), ("method_0", "()I", 1)],
        ),
    ];
    let program = Program::new(&classes, &[]).unwrap();
    let names = NamePlan::build(&program, &RenameOptions::default()).unwrap();
    let first = names.method_name(MemberId { class: 0, index: 0 });
    assert_eq!(first, names.method_name(MemberId { class: 1, index: 0 }));
    assert_eq!(first, names.method_name(MemberId { class: 3, index: 0 }));
    assert_ne!(first, "if");
    assert_ne!(first, "method_0");
}

#[test]
fn return_only_overloads_keep_distinct_descriptor_families() {
    let classes = vec![
        class("First", None, &[], 0x601, &[("a", "()I", 0x401)]),
        class(
            "Second",
            None,
            &[],
            0x601,
            &[("a", "()Ljava/lang/Object;", 0x401)],
        ),
        class(
            "Impl",
            None,
            &["First", "Second"],
            1,
            &[("a", "()I", 1), ("a", "()Ljava/lang/Object;", 1)],
        ),
    ];
    let program = Program::new(&classes, &[]).unwrap();
    let names = NamePlan::build(&program, &RenameOptions::default()).unwrap();
    let integer = names.method_name(MemberId { class: 0, index: 0 });
    let object = names.method_name(MemberId { class: 1, index: 0 });
    assert_ne!(integer, object);
    assert_eq!(integer, names.method_name(MemberId { class: 2, index: 0 }));
    assert_eq!(object, names.method_name(MemberId { class: 2, index: 1 }));
}

#[test]
fn inherited_library_implementation_pins_a_contract_name() {
    let classes = vec![
        class("Contract", None, &[], 0x601, &[("run", "()V", 0x401)]),
        class("Impl", Some("Library"), &["Contract"], 1, &[]),
    ];
    let libraries = vec![class("Library", None, &[], 1, &[("run", "()V", 0x11)])];
    let program = Program::new(&classes, &libraries).unwrap();
    let names = NamePlan::build(&program, &RenameOptions::default()).unwrap();
    assert_eq!(names.method_name(MemberId { class: 0, index: 0 }), "run");
    assert_eq!(
        program.resolve_method("Impl", "run", "()V"),
        Some(MemberId { class: 2, index: 0 })
    );
}

#[test]
fn package_groups_and_portable_type_namespace_are_allocated_together() {
    let classes = vec![
        class("p/if/a", None, &[], 1, &[]),
        class("p/if/b", None, &[], 1, &[]),
        class("p/package_0/Keep", None, &[], 1, &[]),
        class("p/a", None, &[], 1, &[]),
        class("p/a/Child", None, &[], 1, &[]),
        class("p/A", None, &[], 1, &[]),
    ];
    let program = Program::new(&classes, &[]).unwrap();
    let names = NamePlan::build(&program, &RenameOptions::default()).unwrap();
    assert_eq!(
        names.class_name(0).rsplit_once('/').unwrap().0,
        names.class_name(1).rsplit_once('/').unwrap().0
    );
    assert_ne!(
        names.class_name(0).rsplit_once('/').unwrap().0,
        "p/package_0"
    );
    assert_eq!(names.class_name(2), "p/package_0/Keep");
    assert_ne!(names.class_name(3), "p/a");
    let portable: std::collections::HashSet<_> = (0..classes.len())
        .map(|id| names.class_name(id).to_lowercase())
        .collect();
    assert_eq!(portable.len(), classes.len());
}

#[test]
fn field_resolution_uses_jvm_interfaces_before_superclass() {
    let mut classes = vec![
        class("Base", None, &[], 1, &[]),
        class("Face", None, &[], 0x601, &[]),
        class("Child", Some("Base"), &["Face"], 1, &[]),
    ];
    for class in &mut classes[..2] {
        class.fields.push(Field {
            access_flags: 0x19,
            name: "f".into(),
            descriptor: "I".into(),
            constant_value: None,
            synthetic: false,
        });
    }
    let program = Program::new(&classes, &[]).unwrap();
    assert_eq!(
        program.resolve_field("Child", "f", "I"),
        Some(MemberId { class: 1, index: 0 })
    );
}

#[test]
fn unrelated_type_names_do_not_rename_an_entire_field_group() {
    let mut classes = vec![
        class("Fields", None, &[], 1, &[]),
        class("a", None, &[], 1, &[]),
        class("b", None, &[], 1, &[]),
    ];
    classes[0].fields = [("a", "I"), ("a", "J"), ("b", "I")]
        .into_iter()
        .map(|(name, descriptor)| Field {
            access_flags: 1,
            name: name.into(),
            descriptor: descriptor.into(),
            constant_value: None,
            synthetic: false,
        })
        .collect();
    let program = Program::new(&classes, &[]).unwrap();
    let names = NamePlan::build(&program, &RenameOptions::default()).unwrap();
    assert_eq!(names.field_name(MemberId { class: 0, index: 0 }), "a");
    assert_ne!(names.field_name(MemberId { class: 0, index: 1 }), "a");
    assert_eq!(names.field_name(MemberId { class: 0, index: 2 }), "b");
}

#[test]
fn unicode_names_follow_java_categories_and_filesystem_normalization() {
    use sporeflower::naming::is_identifier;
    assert!(is_identifier("€amount"));
    assert!(is_identifier("cafe\u{301}"));
    assert!(!is_identifier("\u{301}cafe"));
    assert!(!is_identifier("value²"));
    assert!(!is_identifier("value\u{200b}"));
    let classes = vec![
        class("p/Café", None, &[], 1, &[]),
        class("p/Cafe\u{301}", None, &[], 1, &[]),
    ];
    let program = Program::new(&classes, &[]).unwrap();
    let names = NamePlan::build(&program, &RenameOptions::default()).unwrap();
    assert!(names.class_name(0).contains("Café"));
    assert!(names.class_name(1).contains("class_"));
}

#[test]
fn unnamed_package_moves_as_a_group_only_when_named_code_needs_it() {
    let classes = vec![
        class("Base", None, &[], 1, &[]),
        class("Peer", None, &[], 1, &[]),
        class("client/Child", Some("Base"), &[], 1, &[]),
        class("defaultpkg/Reserved", None, &[], 1, &[]),
    ];
    let program = Program::new(&classes, &[]).unwrap();
    let names = NamePlan::build(&program, &RenameOptions::default()).unwrap();
    assert_eq!(names.class_name(0), "defaultpkg_0/Base");
    assert_eq!(names.class_name(1), "defaultpkg_0/Peer");
    assert_eq!(names.class_name(2), "client/Child");
    assert_eq!(names.class_name(3), "defaultpkg/Reserved");
    assert_eq!(
        names.package_moves(),
        &[(String::new(), "defaultpkg_0".into())]
    );
}

#[test]
fn authored_names_reserve_targets_and_move_unmapped_package_peers() {
    let classes = vec![
        class("old/First", None, &[], 1, &[]),
        class("old/Peer", None, &[], 1, &[]),
        class("old/Engine", None, &[], 1, &[]),
        class("old/Domain", None, &[], 1, &[]),
    ];
    let program = Program::new(&classes, &[]).unwrap();
    let mut overrides = NameOverrides::default();
    overrides.classes.insert(0, "game/Engine".into());
    overrides.reserved_classes.insert("game/Domain".into());
    let names = NamePlan::build_with_overrides(&program, &Default::default(), &overrides).unwrap();
    assert_eq!(names.class_name(0), "game/Engine");
    assert_eq!(names.class_name(1), "game/Peer");
    assert_ne!(names.class_name(2), "game/Engine");
    assert_ne!(names.class_name(3), "game/Domain");
    assert!(names.class_name(2).starts_with("game/"));
    assert_eq!(names.class_name_for("old/First"), "game/Engine");
    assert_eq!(names.package_moves(), &[("old".into(), "game".into())]);
    assert_eq!(program.class(0).name, "old/First");
    overrides.classes.insert(1, "game/Domain".into());
    assert!(NamePlan::build_with_overrides(&program, &Default::default(), &overrides).is_err());
}

#[test]
fn authored_class_swaps_keep_original_lookup_identities() {
    let classes = vec![
        class("First", None, &[], 1, &[]),
        class("Second", None, &[], 1, &[]),
    ];
    let program = Program::new(&classes, &[]).unwrap();
    let mut overrides = NameOverrides::default();
    overrides.classes.insert(0, "Second".into());
    overrides.classes.insert(1, "First".into());
    let names = NamePlan::build_with_overrides(&program, &Default::default(), &overrides).unwrap();
    assert_eq!(names.class_name_for("First"), "Second");
    assert_eq!(names.class_name_for("Second"), "First");
}

#[test]
fn authored_class_targets_and_package_boundaries_reject_conflicts() {
    let classes = vec![
        class("old/First", None, &[], 1, &[]),
        class("old/Second", None, &[], 1, &[]),
        class("other/Third", None, &[], 1, &[]),
    ];
    let libraries = vec![class("api/External", None, &[], 1, &[])];
    let program = Program::new(&classes, &libraries).unwrap();
    for mappings in [
        vec![(0, "game/Same"), (1, "game/Same")],
        vec![(0, "game/Same"), (1, "game/same")],
        vec![(0, "api/External")],
        vec![(0, "game/class")],
    ] {
        let mut overrides = NameOverrides::default();
        overrides
            .classes
            .extend(mappings.into_iter().map(|(id, name)| (id, name.into())));
        assert!(NamePlan::build_with_overrides(&program, &Default::default(), &overrides).is_err());
    }
}

#[test]
fn authored_placement_allows_independent_package_splits_and_merges() {
    let classes = vec![
        class("old/First", None, &[], 1, &[]),
        class("old/Second", None, &[], 1, &[]),
        class("other/Third", None, &[], 1, &[]),
    ];
    let libraries = vec![class("api/External", None, &[], 1, &[])];
    let program = Program::new(&classes, &libraries).unwrap();
    for mappings in [
        vec![(0, "game/First"), (1, "split/Second")],
        vec![(0, "game/First"), (2, "game/Third")],
        vec![(0, "api/First")],
    ] {
        let mut overrides = NameOverrides::default();
        for &(id, name) in &mappings {
            overrides.classes.insert(id, name.into());
        }
        let names =
            NamePlan::build_with_overrides(&program, &Default::default(), &overrides).unwrap();
        for (id, name) in mappings {
            assert_eq!(names.class_name(id), name);
        }
    }
}

#[test]
fn authored_output_paths_do_not_include_library_files() {
    let classes = vec![class("app/Owner", None, &[], 1, &[])];
    let libraries = vec![
        class("api/media/Control", None, &[], 1, &[]),
        class("api/media/control/Volume", None, &[], 1, &[]),
    ];
    let program = Program::new(&classes, &libraries).unwrap();
    let mut overrides = NameOverrides::default();
    overrides.classes.insert(0, "app/Readable".into());
    overrides.classes.insert(1, "api/media/Control".into());
    overrides
        .reserved_classes
        .insert("api/media/control/Flags".into());
    let names = NamePlan::build_with_overrides(&program, &Default::default(), &overrides).unwrap();
    assert_eq!(names.class_name(0), "app/Readable");
    assert_eq!(names.class_name(1), "api/media/Control");
    assert_eq!(names.class_name(2), "api/media/control/Volume");
    // The same pair of paths is unsafe when both are actually emitted.
    overrides
        .reserved_classes
        .insert("api/media/Control".into());
    assert!(NamePlan::build_with_overrides(&program, &Default::default(), &overrides).is_err());
    overrides.reserved_classes.remove("api/media/Control");
    // Java's exact class/package collisions still apply to classpath types.
    overrides
        .reserved_classes
        .insert("api/media/Control/Flags".into());
    assert!(NamePlan::build_with_overrides(&program, &Default::default(), &overrides).is_err());
}

fn field_read(class: &mut ClassFile, owner: &str, flags: u16, descriptor: &str, bytes: &[u8]) {
    use sporeflower::classfile::{Code, Constant};
    class.constant_pool.entries = vec![
        Constant::Unusable,
        Constant::Utf8(owner.into()),
        Constant::Class(1),
        Constant::Utf8("value".into()),
        Constant::Utf8("I".into()),
        Constant::NameAndType {
            name_index: 3,
            descriptor_index: 4,
        },
        Constant::Fieldref {
            class_index: 2,
            name_and_type_index: 5,
        },
    ];
    class.methods.push(Method {
        name: "read".into(),
        descriptor: descriptor.into(),
        access_flags: flags,
        code: Some(Code {
            max_stack: 1,
            max_locals: 1,
            bytes: bytes.into(),
            exception_table: Vec::new(),
            line_numbers: Vec::new(),
            local_variables: Vec::new(),
        }),
        exceptions: Vec::new(),
        synthetic: false,
    });
}

fn integer_field(flags: u16) -> Field {
    Field {
        name: "value".into(),
        descriptor: "I".into(),
        access_flags: flags,
        constant_value: None,
        synthetic: false,
    }
}

#[test]
fn authored_split_checks_actual_package_member_accesses() {
    for used in [false, true] {
        let mut classes = vec![
            class("old/Owner", None, &[], 1, &[]),
            class("old/Reader", None, &[], 1, &[]),
        ];
        classes[0].fields.push(integer_field(8));
        field_read(
            &mut classes[1],
            "old/Owner",
            9,
            if used { "()I" } else { "()V" },
            if used { &[0xb2, 0, 6, 0xac] } else { &[0xb1] },
        );
        let program = Program::new(&classes, &[]).unwrap();
        let mut overrides = NameOverrides::default();
        overrides.classes.insert(0, "data/Owner".into());
        overrides.classes.insert(1, "client/Reader".into());
        let result = NamePlan::build_with_overrides(&program, &Default::default(), &overrides);
        if used {
            let error = result.unwrap_err().to_string();
            assert!(
                error.contains("package access") && error.contains("old/Owner.value"),
                "{error}"
            );
        } else {
            result.unwrap();
        }
    }
}

#[test]
fn authored_split_checks_package_private_types_and_overrides() {
    let mut classes = vec![
        class("old/Base", None, &[], 0, &[("run", "()V", 0)]),
        class("old/Child", Some("old/Base"), &[], 1, &[("run", "()V", 1)]),
    ];
    let mut overrides = NameOverrides::default();
    overrides.classes.insert(0, "base/Base".into());
    overrides.classes.insert(1, "client/Child".into());
    let error = NamePlan::build_with_overrides(
        &Program::new(&classes, &[]).unwrap(),
        &Default::default(),
        &overrides,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("package-private class"), "{error}");
    classes[0].access_flags = 1;
    let error = NamePlan::build_with_overrides(
        &Program::new(&classes, &[]).unwrap(),
        &Default::default(),
        &overrides,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("package-private override"), "{error}");
    classes[0].methods[0].access_flags = 4;
    NamePlan::build_with_overrides(
        &Program::new(&classes, &[]).unwrap(),
        &Default::default(),
        &overrides,
    )
    .unwrap();
}

#[test]
fn authored_merge_keeps_previously_distinct_dispatch_families_distinct() {
    let classes = vec![
        class("base/Base", None, &[], 1, &[("run", "()V", 0)]),
        class(
            "child/Child",
            Some("base/Base"),
            &[],
            1,
            &[("run", "()V", 1)],
        ),
    ];
    let program = Program::new(&classes, &[]).unwrap();
    let mut overrides = NameOverrides::default();
    overrides.classes.insert(0, "merged/Base".into());
    overrides.classes.insert(1, "merged/Child".into());
    let names = NamePlan::build_with_overrides(&program, &Default::default(), &overrides).unwrap();
    assert_ne!(
        names.method_name(MemberId { class: 0, index: 0 }),
        names.method_name(MemberId { class: 1, index: 0 })
    );
    for class in 0..2 {
        overrides
            .methods
            .insert(MemberId { class, index: 0 }, "run".into());
    }
    assert!(NamePlan::build_with_overrides(&program, &Default::default(), &overrides).is_err());
}

#[test]
fn authored_split_preserves_only_legal_protected_receivers() {
    for receiver in ["Base", "Child"] {
        let mut classes = vec![
            class("old/Base", None, &[], 1, &[]),
            class("old/Child", Some("old/Base"), &[], 1, &[]),
        ];
        classes[0].fields.push(integer_field(4));
        field_read(
            &mut classes[1],
            "old/Base",
            9,
            &format!("(Lold/{receiver};)I"),
            &[0x2a, 0xb4, 0, 6, 0xac],
        );
        let program = Program::new(&classes, &[]).unwrap();
        let mut overrides = NameOverrides::default();
        overrides.classes.insert(0, "base/Base".into());
        overrides.classes.insert(1, "child/Child".into());
        let result = NamePlan::build_with_overrides(&program, &Default::default(), &overrides);
        if receiver == "Base" {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("protected receiver")
            );
        } else {
            result.unwrap();
        }
    }
}

#[test]
fn authored_split_checks_the_final_qualifier_of_hidden_protected_fields() {
    for (this_receiver, rename_child) in [(true, false), (false, false), (false, true)] {
        let mut classes = vec![
            class("old/Base", None, &[], 1, &[]),
            class("old/Child", Some("old/Base"), &[], 1, &[]),
        ];
        classes[0].fields.push(integer_field(4));
        classes[1].fields.push(integer_field(1));
        field_read(
            &mut classes[1],
            "old/Base",
            if this_receiver { 1 } else { 9 },
            if this_receiver {
                "()I"
            } else {
                "(Lold/Child;)I"
            },
            &[0x2a, 0xb4, 0, 6, 0xac],
        );
        let program = Program::new(&classes, &[]).unwrap();
        let mut overrides = NameOverrides::default();
        overrides.classes.insert(0, "base/Base".into());
        overrides.classes.insert(1, "child/Child".into());
        if rename_child {
            overrides
                .fields
                .insert(MemberId { class: 1, index: 0 }, "ownValue".into());
        }
        let result = NamePlan::build_with_overrides(&program, &Default::default(), &overrides);
        if !this_receiver && !rename_child {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("protected receiver")
            );
        } else {
            result.unwrap();
        }
    }
}

#[test]
fn split_package_moves_retain_every_original_and_emitted_pair() {
    let classes = vec![
        class("old/First", None, &[], 1, &[]),
        class("old/Second", None, &[], 1, &[]),
    ];
    let program = Program::new(&classes, &[]).unwrap();
    let mut overrides = NameOverrides::default();
    overrides.classes.insert(0, "a/First".into());
    overrides.classes.insert(1, "b/Second".into());
    let names = NamePlan::build_with_overrides(&program, &Default::default(), &overrides).unwrap();
    assert_eq!(
        names.package_moves(),
        &[("old".into(), "a".into()), ("old".into(), "b".into())]
    );
    let mut identities: Vec<_> = names.class_mappings().collect();
    identities.sort_unstable();
    assert_eq!(
        identities,
        vec![("old/First", "a/First"), ("old/Second", "b/Second")]
    );
}

#[test]
fn authored_method_names_propagate_through_virtual_families() {
    let classes = vec![
        class("Contract", None, &[], 0x601, &[("a", "()I", 0x401)]),
        class(
            "Impl",
            None,
            &["Contract"],
            1,
            &[("a", "()I", 1), ("read", "()I", 1)],
        ),
    ];
    let program = Program::new(&classes, &[]).unwrap();
    let mut overrides = NameOverrides::default();
    let contract = MemberId { class: 0, index: 0 };
    let implementation = MemberId { class: 1, index: 0 };
    overrides.methods.insert(implementation, "read".into());
    let names = NamePlan::build_with_overrides(&program, &Default::default(), &overrides).unwrap();
    assert_eq!(names.method_name(contract), "read");
    assert_eq!(names.method_name(implementation), "read");
    assert_ne!(names.method_name(MemberId { class: 1, index: 1 }), "read");
    assert_eq!(
        names.resolved_method_name(&program, "Impl", "a", "()I"),
        "read"
    );
    overrides.methods.insert(contract, "fetch".into());
    assert!(NamePlan::build_with_overrides(&program, &Default::default(), &overrides).is_err());
}

#[test]
fn authored_methods_cannot_change_external_or_native_contracts() {
    let classes = vec![class(
        "Impl",
        Some("api/Base"),
        &[],
        1,
        &[("run", "()V", 1), ("nativeCall", "()V", 0x101)],
    )];
    let libraries = vec![class("api/Base", None, &[], 1, &[("run", "()V", 1)])];
    let program = Program::new(&classes, &libraries).unwrap();
    for index in 0..2 {
        let mut overrides = NameOverrides::default();
        overrides
            .methods
            .insert(MemberId { class: 0, index }, "renamed".into());
        assert!(NamePlan::build_with_overrides(&program, &Default::default(), &overrides).is_err());
    }
}

#[test]
fn authored_fields_take_precedence_without_capturing_inherited_accesses() {
    let mut classes = vec![
        class("Base", None, &[], 1, &[]),
        class("Child", Some("Base"), &[], 1, &[]),
    ];
    for (owner, descriptor) in [(0, "I"), (1, "J")] {
        classes[owner].fields.push(Field {
            access_flags: 1,
            name: "value".into(),
            descriptor: descriptor.into(),
            constant_value: None,
            synthetic: false,
        });
    }
    let program = Program::new(&classes, &[]).unwrap();
    let mut overrides = NameOverrides::default();
    overrides
        .fields
        .insert(MemberId { class: 1, index: 0 }, "value".into());
    let names = NamePlan::build_with_overrides(&program, &Default::default(), &overrides).unwrap();
    assert_eq!(names.field_name(MemberId { class: 1, index: 0 }), "value");
    assert_ne!(names.field_name(MemberId { class: 0, index: 0 }), "value");
    overrides
        .fields
        .insert(MemberId { class: 0, index: 0 }, "value".into());
    assert!(NamePlan::build_with_overrides(&program, &Default::default(), &overrides).is_err());
}

#[test]
fn authored_same_type_field_cannot_capture_a_different_inherited_identity() {
    let mut classes = vec![
        class("p/Base", None, &[], 0, &[]),
        class("p/Child", Some("p/Base"), &[], 1, &[]),
    ];
    for (owner, name) in [(0, "a"), (1, "b")] {
        classes[owner].fields.push(Field {
            access_flags: 0x0009,
            name: name.into(),
            descriptor: "I".into(),
            constant_value: None,
            synthetic: false,
        });
    }
    let program = Program::new(&classes, &[]).unwrap();
    let mut overrides = NameOverrides::default();
    overrides
        .fields
        .insert(MemberId { class: 1, index: 0 }, "a".into());
    let names = NamePlan::build_with_overrides(&program, &Default::default(), &overrides).unwrap();
    assert_eq!(names.field_name(MemberId { class: 1, index: 0 }), "a");
    assert_ne!(names.field_name(MemberId { class: 0, index: 0 }), "a");
    overrides
        .fields
        .insert(MemberId { class: 0, index: 0 }, "a".into());
    assert!(NamePlan::build_with_overrides(&program, &Default::default(), &overrides).is_err());
}

#[test]
fn authored_unnamed_package_targets_cannot_break_named_source_references() {
    for original in ["old/Base", "Base"] {
        let classes = vec![
            class(original, None, &[], 1, &[]),
            class("client/Child", Some(original), &[], 1, &[]),
        ];
        let program = Program::new(&classes, &[]).unwrap();
        let mut overrides = NameOverrides::default();
        overrides.classes.insert(0, "Base".into());
        let error =
            NamePlan::build_with_overrides(&program, &Default::default(), &overrides).unwrap_err();
        assert!(error.to_string().contains("unnamed package"), "{error}");
        overrides.classes.insert(0, "readable/Base".into());
        assert!(NamePlan::build_with_overrides(&program, &Default::default(), &overrides).is_ok());
    }
}

#[test]
fn authored_parameters_use_ordinals_and_reject_invalid_requests() {
    let classes = vec![class("Owner", None, &[], 1, &[("a", "(JDI)V", 1)])];
    let program = Program::new(&classes, &[]).unwrap();
    let member = MemberId { class: 0, index: 0 };
    let mut overrides = NameOverrides::default();
    overrides.parameters.insert(
        member,
        ["timestamp", "scale", "count"].map(Into::into).to_vec(),
    );
    let names = NamePlan::build_with_overrides(&program, &Default::default(), &overrides).unwrap();
    assert_eq!(names.parameter_name(member, 0), Some("timestamp"));
    assert_eq!(names.parameter_name(member, 1), Some("scale"));
    assert_eq!(names.parameter_name(member, 2), Some("count"));
    assert_eq!(names.parameter_name(member, 3), None);
    for names in [
        vec!["only"],
        vec!["same", "same", "count"],
        vec!["class", "scale", "count"],
        vec!["$state", "scale", "count"],
    ] {
        overrides
            .parameters
            .insert(member, names.into_iter().map(Into::into).collect());
        assert!(NamePlan::build_with_overrides(&program, &Default::default(), &overrides).is_err());
    }
}
