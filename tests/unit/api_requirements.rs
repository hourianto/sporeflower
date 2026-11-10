use super::*;
use crate::classfile::{Code, ConstantPool, Field, Method};

fn class(name: &str) -> ClassFile {
    let mut constant_pool = ConstantPool::default();
    constant_pool.entries.push(Constant::Unusable);
    ClassFile {
        minor_version: 0,
        major_version: 49,
        access_flags: 1,
        name: name.into(),
        super_name: None,
        interfaces: vec![],
        constant_pool,
        fields: vec![],
        methods: vec![],
        source_file: None,
        inner_classes: vec![],
        warnings: vec![],
    }
}

fn reference(class: &mut ClassFile, owner: &str, name: &str, descriptor: &str, tag: u8) -> u16 {
    let entries = &mut class.constant_pool.entries;
    let start = entries.len() as u16;
    entries.extend([
        Constant::Utf8(owner.into()),
        Constant::Class(start),
        Constant::Utf8(name.into()),
        Constant::Utf8(descriptor.into()),
        Constant::NameAndType {
            name_index: start + 2,
            descriptor_index: start + 3,
        },
    ]);
    let (class_index, name_and_type_index) = (start + 1, start + 4);
    entries.push(match tag {
        9 => Constant::Fieldref {
            class_index,
            name_and_type_index,
        },
        10 => Constant::Methodref {
            class_index,
            name_and_type_index,
        },
        11 => Constant::InterfaceMethodref {
            class_index,
            name_and_type_index,
        },
        _ => unreachable!(),
    });
    start + 5
}

fn uses(class: &mut ClassFile, instructions: &[(u8, u16)]) {
    let mut bytes = Vec::new();
    for &(opcode, index) in instructions {
        bytes.push(opcode);
        bytes.extend(index.to_be_bytes());
        if opcode == 0xb9 {
            bytes.extend([1, 0]);
        }
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
fn unused_descriptors_gain_signature_evidence_only_when_declared_or_invoked() {
    let descriptor = "([Ltypes/Argument;D)Ltypes/Result;";
    let mut first = class("First");
    reference(&mut first, "api/One", "unused", descriptor, 10);
    reference(&mut first, "api/One", "field", "[Ltypes/Field;", 9);
    let mut second = class("Second");
    let invoked = reference(&mut second, "api/Two", "used", descriptor, 10);
    let mut project = [first, second];
    let weak = Requirements::inspect(&project);
    assert!(weak.signature_types.is_empty());
    assert!(weak.floating_point);
    for name in ["types/Argument", "types/Result", "types/Field"] {
        assert!(weak.classes.contains(name));
    }

    uses(&mut project[1], &[(0xb8, invoked), (0xb8, invoked)]);
    let invoked = Requirements::inspect(&project);
    assert_eq!(
        invoked.signature_types,
        BTreeSet::from(["types/Argument".into(), "types/Result".into()])
    );
    assert!(invoked.classes.contains("types/Field"));
    assert_eq!(invoked.members["api/One"].len(), 2);
    assert_eq!(invoked.members["api/Two"].len(), 1);

    project[1].fields.push(Field {
        access_flags: 1,
        name: "declared".into(),
        descriptor: "[Ltypes/Field;".into(),
        constant_value: None,
        synthetic: false,
    });
    let declared = Requirements::inspect(&project);
    project.reverse();
    let reversed = Requirements::inspect(&project);
    assert!(declared.signature_types.contains("types/Field"));
    assert_eq!(declared.classes, reversed.classes);
    assert_eq!(declared.signature_types, reversed.signature_types);
    assert_eq!(declared.members, reversed.members);
}

#[test]
fn malformed_descriptors_never_contribute_partial_types_when_promoted() {
    let mut owner = class("Owner");
    let descriptor = "(Ltypes/Partial;D)Vtrailing";
    let index = reference(&mut owner, "api/Api", "invalid", descriptor, 10);
    uses(&mut owner, &[(0xb8, index), (0xb8, index)]);
    owner.methods[0].descriptor = descriptor.into();
    let result = Requirements::inspect(&[owner]);
    assert_eq!(result.classes, BTreeSet::from(["api/Api".into()]));
    assert!(result.signature_types.is_empty());
    assert!(!result.floating_point);
    assert!(result.warnings.is_empty());
}

#[test]
fn shared_descriptors_keep_distinct_members_and_all_invocation_kinds() {
    let mut owner = class("Owner");
    let method = reference(&mut owner, "api/Api", "call", "()Ltypes/Result;", 10);
    let interface = reference(&mut owner, "api/Api", "call", "()Ltypes/Result;", 11);
    let other = reference(&mut owner, "api/Api", "other", "()Ltypes/Result;", 10);
    let field = reference(&mut owner, "api/Api", "field", "Ltypes/Field;", 9);
    uses(
        &mut owner,
        &[
            (0xb6, method),
            (0xb9, interface),
            (0xb8, method),
            (0xb8, other),
            (0xb2, field),
            (0xb3, field),
            (0xb4, field),
            (0xb5, field),
        ],
    );
    uses(&mut owner, &[(0xb8, method), (0xb9, interface)]);
    let result = Requirements::inspect(&[owner]);
    assert_eq!(
        result.interface_owners["api/Api"],
        BTreeSet::from([false, true])
    );
    assert!(result.invoked_classes.contains("api/Api"));
    assert!(result.invoked_interfaces.contains("api/Api"));
    for name in ["call", "field"] {
        assert_eq!(
            result.members["api/Api"]
                .iter()
                .filter(|member| member.name == name)
                .map(|member| member.is_static)
                .collect::<Vec<_>>(),
            [Some(false), Some(true)]
        );
    }
    assert_eq!(result.members["api/Api"].len(), 5);
    assert_eq!(
        result.signature_types,
        BTreeSet::from(["types/Result".into(), "types/Field".into()])
    );
}

#[test]
fn constant_indices_are_local_to_each_class_and_invalid_references_are_ignored() {
    let mut project = [class("First"), class("Second")];
    for (class, owner) in project.iter_mut().zip(["api/One", "api/Two"]) {
        let index = reference(class, owner, "call", "()V", 10);
        uses(
            class,
            &[
                (0xbb, index - 4),
                (0xb8, index),
                (0xbb, u16::MAX),
                (0xb8, u16::MAX),
            ],
        );
        // Decode failure remains per-method evidence; it must not be cached away.
        let mut invalid = class.methods[0].clone();
        invalid.name = "invalid".into();
        invalid.code.as_mut().unwrap().bytes = vec![0xff];
        class.methods.push(invalid);
    }
    let result = Requirements::inspect(&project);
    assert_eq!(
        result.instantiated_classes,
        BTreeSet::from(["api/One".into(), "api/Two".into()])
    );
    assert_eq!(result.invoked_classes, result.instantiated_classes);
    assert_eq!(result.members.len(), 2);
    assert_eq!(result.warnings.len(), 2);
    assert!(result.warnings[0].contains("First.invalid()V"));
    assert!(result.warnings[1].contains("Second.invalid()V"));
}
