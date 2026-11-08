use super::*;
use crate::classfile::{ClassFile, Code, ConstantPool, Field, Method, STATIC};
use crate::program::AnalysisScope;

fn application<'a>(classes: &'a [ClassFile], libraries: &'a [ClassFile]) -> Program<'a> {
    Program::with_scope(classes, libraries, AnalysisScope::Application).unwrap()
}

fn fixture() -> ClassFile {
    ClassFile {
        minor_version: 0,
        major_version: 49,
        access_flags: 1,
        name: "Flags".into(),
        super_name: Some("java/lang/Object".into()),
        interfaces: vec![],
        constant_pool: ConstantPool::default(),
        fields: vec![],
        methods: vec![],
        source_file: None,
        inner_classes: vec![],
        warnings: vec![],
    }
}

fn reference(class: &mut ClassFile, owner: &str, name: &str, descriptor: &str, field: bool) -> u16 {
    let entries = &mut class.constant_pool.entries;
    if entries.is_empty() {
        entries.push(Constant::Unusable);
    }
    let first = entries.len() as u16;
    entries.extend([
        Constant::Utf8(owner.into()),
        Constant::Class(first),
        Constant::Utf8(name.into()),
        Constant::Utf8(descriptor.into()),
        Constant::NameAndType {
            name_index: first + 2,
            descriptor_index: first + 3,
        },
    ]);
    entries.push(if field {
        Constant::Fieldref {
            class_index: first + 1,
            name_and_type_index: first + 4,
        }
    } else {
        Constant::Methodref {
            class_index: first + 1,
            name_and_type_index: first + 4,
        }
    });
    first + 5
}

fn field(class: &mut ClassFile, name: &str, access_flags: u16) -> u16 {
    class.fields.push(Field {
        name: name.into(),
        descriptor: "Z".into(),
        access_flags,
        constant_value: None,
        synthetic: false,
    });
    reference(class, "Flags", name, "Z", true)
}

fn method(
    class: &mut ClassFile,
    name: &str,
    descriptor: &str,
    flags: u16,
    bytes: Vec<u8>,
) -> MemberId {
    let member = MemberId {
        class: 0,
        index: class.methods.len(),
    };
    class.methods.push(Method {
        name: name.into(),
        descriptor: descriptor.into(),
        access_flags: flags,
        exceptions: vec![],
        synthetic: false,
        code: Some(Code {
            max_stack: 6,
            max_locals: 4,
            bytes,
            exception_table: vec![],
            line_numbers: vec![],
            local_variables: vec![],
        }),
    });
    member
}

fn instruction(bytes: &mut Vec<u8>, opcode: u8, reference: u16) {
    bytes.push(opcode);
    bytes.extend(reference.to_be_bytes());
}

fn certified(program: &Program<'_>, name: &str) -> bool {
    program.canonical_boolean_field(&Member {
        owner: "Flags".into(),
        name: name.into(),
        descriptor: "Z".into(),
    })
}

#[test]
fn nested_eager_masks_bound_raw_return_producers() {
    let mut class = fixture();
    method(
        &mut class,
        "masked",
        "(III)Z",
        1 | STATIC,
        // ((first & second) & 1) & third stays 0/1 for arbitrary inputs.
        vec![0x1a, 0x1b, 0x7e, 0x04, 0x7e, 0x1c, 0x7e, 0xac],
    );
    method(
        &mut class,
        "unmasked",
        "(III)Z",
        1 | STATIC,
        vec![0x1a, 0x1b, 0x7e, 0x1c, 0x7e, 0xac],
    );
    let classes = [class];
    let program = Program::new(&classes, &[]).unwrap();
    for (name, expected) in [("masked", true), ("unmasked", false)] {
        assert_eq!(
            program.canonical_boolean_return(
                None,
                InvokeKind::Static,
                &Member {
                    owner: "Flags".into(),
                    name: name.into(),
                    descriptor: "(III)Z".into(),
                },
            ),
            expected,
            "{name}"
        );
    }
}

#[test]
fn application_writes_and_public_arguments_follow_feedback_and_descriptor_indexes() {
    let mut class = fixture();
    let selected = field(&mut class, "selected", 1 | STATIC);
    let saved = field(&mut class, "saved", 1 | STATIC);
    field(&mut class, "external", 1 | STATIC);
    field(&mut class, "privateDefault", PRIVATE | STATIC);
    let set = reference(&mut class, "Flags", "set", "(JZ)V", false);
    let mut setter = vec![0x1c]; // Boolean descriptor index 1 is JVM slot 2.
    instruction(&mut setter, 0xb3, selected);
    setter.push(0xb1);
    let target = method(&mut class, "set", "(JZ)V", 1 | STATIC, setter);
    let mut caller = vec![0x09, 0x03];
    instruction(&mut caller, 0xb8, set);
    caller.extend([0x09, 0x04]);
    instruction(&mut caller, 0xb8, set);
    caller.push(0xb1);
    method(&mut class, "choose", "()V", 1 | STATIC, caller);
    let mut feedback = vec![];
    instruction(&mut feedback, 0xb2, selected);
    instruction(&mut feedback, 0xb3, saved);
    instruction(&mut feedback, 0xb2, saved);
    instruction(&mut feedback, 0xb3, selected);
    feedback.push(0xb1);
    method(&mut class, "feedback", "()V", 1 | STATIC, feedback);
    let classes = [class];
    let program = application(&classes, &[]);
    assert!(certified(&program, "selected"));
    assert!(certified(&program, "saved"));
    assert!(certified(&program, "privateDefault"));
    assert!(certified(&program, "external"));
    assert!(program.canonical_boolean_parameter(target, 1));
    assert!(!program.canonical_boolean_parameter(target, 0));
    assert!(!program.canonical_boolean_parameter(target, 2));
}

#[test]
fn raw_caller_invalidates_public_argument_and_copied_field_cycle() {
    let mut class = fixture();
    let left = field(&mut class, "left", 1 | STATIC);
    let right = field(&mut class, "right", 1 | STATIC);
    let set = reference(&mut class, "Flags", "set", "(Z)V", false);
    let mut setter = vec![0x1a];
    instruction(&mut setter, 0xb3, left);
    setter.push(0xb1);
    let target = method(&mut class, "set", "(Z)V", 1 | STATIC, setter);
    let mut caller = vec![0x04];
    instruction(&mut caller, 0xb8, set);
    caller.push(0x05); // One raw 2 taints all inputs, despite a known 1 caller.
    instruction(&mut caller, 0xb8, set);
    instruction(&mut caller, 0xb2, left);
    instruction(&mut caller, 0xb3, right);
    instruction(&mut caller, 0xb2, right);
    instruction(&mut caller, 0xb3, left);
    caller.push(0xb1);
    method(&mut class, "raw", "()V", 1 | STATIC, caller);
    let classes = [class];
    let program = application(&classes, &[]);
    assert!(!program.canonical_boolean_parameter(target, 0));
    assert!(!certified(&program, "left"));
    assert!(!certified(&program, "right"));
}

#[test]
fn final_stream_contract_does_not_extend_to_arbitrary_methods_or_unobserved_inputs() {
    let mut class = fixture();
    for (field_name, owner) in [
        ("stream", "java/io/DataInputStream"),
        ("unknown", "OtherInput"),
    ] {
        let target = field(&mut class, field_name, 1 | STATIC);
        let read = reference(&mut class, owner, "readBoolean", "()Z", false);
        let mut bytes = vec![0x2a];
        instruction(&mut bytes, 0xb6, read);
        instruction(&mut bytes, 0xb3, target);
        bytes.push(0xb1);
        method(
            &mut class,
            field_name,
            &format!("(L{owner};)V"),
            1 | STATIC,
            bytes,
        );
    }
    let target = field(&mut class, "argument", 1 | STATIC);
    let mut bytes = vec![0x1a];
    instruction(&mut bytes, 0xb3, target);
    bytes.push(0xb1);
    let public = method(&mut class, "publicInput", "(Z)V", 1 | STATIC, bytes);
    let classes = [class];
    let program = application(&classes, &[]);
    assert!(certified(&program, "stream"));
    assert!(!certified(&program, "unknown"));
    assert!(!certified(&program, "argument"));
    assert!(!program.canonical_boolean_parameter(public, 0));
}

#[test]
fn branch_joins_require_every_input_and_unliftable_writers_stay_unknown() {
    let mut class = fixture();
    for (name, other) in [("joined", 0x04), ("rawJoined", 0x05)] {
        let target = field(&mut class, name, 1 | STATIC);
        // iload_0; ifeq false; iconst_0; goto store; iconst_1/2; putstatic
        let mut bytes = vec![0x1a, 0x99, 0x00, 0x07, 0x03, 0xa7, 0x00, 0x04, other];
        instruction(&mut bytes, 0xb3, target);
        bytes.push(0xb1);
        method(&mut class, name, "(I)V", 1 | STATIC, bytes);
    }
    let target = field(&mut class, "failed", PRIVATE | STATIC);
    let mut bytes = vec![0x57, 0x04]; // Invalid pop prevents producer lifting.
    instruction(&mut bytes, 0xb3, target);
    bytes.push(0xb1);
    method(&mut class, "failed", "()V", 1 | STATIC, bytes);
    let classes = [class];
    let program = application(&classes, &[]);
    assert!(certified(&program, "joined"));
    assert!(!certified(&program, "rawJoined"));
    assert!(!certified(&program, "failed"));
}

#[test]
fn exact_returns_share_field_and_private_argument_dependencies() {
    let mut class = fixture();
    let saved = field(&mut class, "saved", PRIVATE | STATIC);
    let pass = reference(&mut class, "Flags", "pass", "(Z)Z", false);
    method(
        &mut class,
        "pass",
        "(Z)Z",
        PRIVATE | STATIC,
        vec![0x1a, 0xac],
    );
    let mut seed = vec![0x04];
    instruction(&mut seed, 0xb8, pass);
    instruction(&mut seed, 0xb3, saved);
    seed.push(0xb1);
    method(&mut class, "seed", "()V", 1 | STATIC, seed);
    let mut getter = vec![];
    instruction(&mut getter, 0xb2, saved);
    getter.push(0xac);
    method(&mut class, "get", "()Z", 1 | STATIC, getter.clone());
    method(&mut class, "closedGet", "()Z", 1 | 0x0010, getter.clone());
    method(&mut class, "openGet", "()Z", 1, getter);
    let get = reference(&mut class, "Flags", "get", "()Z", false);
    let mut wrapper = vec![];
    instruction(&mut wrapper, 0xb8, get);
    wrapper.push(0xac);
    method(&mut class, "wrapper", "()Z", 1 | STATIC, wrapper);
    method(&mut class, "raw", "(Z)Z", 1 | STATIC, vec![0x1a, 0xac]);
    let classes = [class];
    let program = application(&classes, &[]);
    for (name, descriptor, kind, expected) in [
        ("pass", "(Z)Z", super::super::InvokeKind::Static, true),
        ("get", "()Z", super::super::InvokeKind::Static, true),
        ("wrapper", "()Z", super::super::InvokeKind::Static, true),
        ("raw", "(Z)Z", super::super::InvokeKind::Static, false),
        ("closedGet", "()Z", super::super::InvokeKind::Virtual, true),
        ("openGet", "()Z", super::super::InvokeKind::Virtual, true),
    ] {
        assert_eq!(
            program.canonical_boolean_return(
                None,
                kind,
                &Member {
                    owner: "Flags".into(),
                    name: name.into(),
                    descriptor: descriptor.into()
                }
            ),
            expected,
            "{name}"
        );
    }
}

#[test]
fn platform_predicates_need_known_contracts_and_no_supplied_override() {
    use super::super::InvokeKind;
    let mut class = fixture();
    let classes = [class.clone()];
    let program = application(&classes, &[]);
    for (owner, name, descriptor, expected) in [
        ("java/lang/String", "equals", "(Ljava/lang/Object;)Z", true),
        (
            "java/lang/String",
            "endsWith",
            "(Ljava/lang/String;)Z",
            true,
        ),
        (
            "java/util/Vector",
            "contains",
            "(Ljava/lang/Object;)Z",
            false,
        ),
        (
            "java/util/Hashtable",
            "containsKey",
            "(Ljava/lang/Object;)Z",
            false,
        ),
        ("java/lang/String", "unknown", "()Z", false),
        (
            "javax/bluetooth/UUID",
            "equals",
            "(Ljava/lang/Object;)Z",
            false,
        ),
        (
            "javax/bluetooth/RemoteDevice",
            "equals",
            "(Ljava/lang/Object;)Z",
            false,
        ),
        (
            "javax/bluetooth/RemoteDevice",
            "isAuthenticated",
            "()Z",
            false,
        ),
        ("java/lang/Boolean", "booleanValue", "()Z", false),
        ("java/io/DataInput", "readBoolean", "()Z", false),
        ("OtherInput", "readBoolean", "()Z", false),
    ] {
        assert_eq!(
            program.canonical_boolean_return(
                None,
                InvokeKind::Virtual,
                &Member {
                    owner: owner.into(),
                    name: name.into(),
                    descriptor: descriptor.into()
                }
            ),
            expected,
            "{owner}.{name}"
        );
    }
    class.super_name = Some("java/util/Vector".into());
    method(
        &mut class,
        "contains",
        "(Ljava/lang/Object;)Z",
        1,
        vec![0x05, 0xac],
    );
    let classes = [class.clone()];
    let program = application(&classes, &[]);
    assert!(!program.canonical_boolean_return(
        None,
        InvokeKind::Virtual,
        &Member {
            owner: "java/util/Vector".into(),
            name: "contains".into(),
            descriptor: "(Ljava/lang/Object;)Z".into()
        }
    ));

    class.name = "RawVector".into();
    let mut inherited = fixture();
    inherited.super_name = Some(class.name.clone());
    let classes = [inherited];
    let mut base = fixture();
    base.name = "java/util/Vector".into();
    method(
        &mut base,
        "contains",
        "(Ljava/lang/Object;)Z",
        1,
        vec![0x04, 0xac],
    );
    for (libraries, expected) in [(vec![base.clone()], true), (vec![base, class], false)] {
        let program = application(&classes, &libraries);
        assert_eq!(
            program.canonical_boolean_return(
                None,
                InvokeKind::Virtual,
                &Member {
                    owner: "java/util/Vector".into(),
                    name: "contains".into(),
                    descriptor: "(Ljava/lang/Object;)Z".into(),
                },
            ),
            expected,
        );
    }
}

#[test]
fn supplied_virtual_implementations_share_one_return_proof_and_raw_overrides_invalidate_it() {
    use super::super::InvokeKind;
    let mut base = fixture();
    base.access_flags |= 0x0400;
    method(&mut base, "check", "()Z", 1, vec![0x03, 0xac]);
    let check = reference(&mut base, "Flags", "check", "()Z", false);
    let mut caller = vec![0x2a];
    instruction(&mut caller, 0xb6, check);
    caller.push(0xac);
    method(&mut base, "caller", "(LFlags;)Z", 1 | STATIC, caller);
    let mut child = fixture();
    child.name = "Child".into();
    child.super_name = Some("Flags".into());
    method(&mut child, "check", "()Z", 1, vec![0x04, 0xac]);
    let mut other = child.clone();
    other.name = "Other".into();
    let mut classes = [base, child, other];
    for expected in [true, false] {
        let program = application(&classes, &[]);
        for (name, descriptor, kind) in [
            ("check", "()Z", InvokeKind::Virtual),
            ("caller", "(LFlags;)Z", InvokeKind::Static),
        ] {
            assert_eq!(
                program.canonical_boolean_return(
                    None,
                    kind,
                    &Member {
                        owner: "Flags".into(),
                        name: name.into(),
                        descriptor: descriptor.into()
                    }
                ),
                expected,
                "{name}"
            );
        }
        classes[2].methods[0].code.as_mut().unwrap().bytes = vec![0x05, 0xac];
    }
}

#[test]
fn virtual_library_implementation_does_not_inherit_an_application_interface_proof() {
    use super::super::InvokeKind;
    let mut interface = fixture();
    interface.access_flags |= 0x0600;
    method(&mut interface, "check", "()Z", 1 | 0x0400, vec![]);
    interface.methods[0].code = None;
    let check = reference(&mut interface, "Flags", "check", "()Z", false);
    let mut caller = vec![0x2a];
    instruction(&mut caller, 0xb9, check);
    caller.extend([1, 0, 0xac]);
    method(&mut interface, "caller", "(LFlags;)Z", 1 | STATIC, caller);
    let mut child = fixture();
    child.name = "Child".into();
    child.super_name = Some("Provider".into());
    child.interfaces.push("Flags".into());
    let mut provider = fixture();
    provider.name = "Provider".into();
    method(&mut provider, "check", "()Z", 1, vec![0x04, 0xac]);
    let classes = [interface, child];
    let libraries = [provider];
    let program = application(&classes, &libraries);
    assert!(!program.canonical_boolean_return(
        None,
        InvokeKind::Interface,
        &Member {
            owner: "Flags".into(),
            name: "check".into(),
            descriptor: "()Z".into()
        }
    ));
}

#[test]
fn api_base_dispatch_checks_recursive_application_and_library_overrides() {
    let descriptor = "(Ljava/lang/Object;)Z";
    let object_equals = Member {
        owner: "java/lang/Object".into(),
        name: "equals".into(),
        descriptor: descriptor.into(),
    };
    let mut class = fixture();
    class.access_flags |= 0x0020;
    let equals = reference(
        &mut class,
        &object_equals.owner,
        "equals",
        descriptor,
        false,
    );
    // Recursive virtual forwarding and an exact super fallback share one proof.
    let mut bytes = vec![0x2b, 0xc6, 0, 9, 0x2b, 0x2a];
    instruction(&mut bytes, 0xb6, equals);
    bytes.extend([0xac, 0x2a, 0x2b]);
    instruction(&mut bytes, 0xb7, equals);
    bytes.push(0xac);
    method(&mut class, "equals", descriptor, 1, bytes);
    let mut other = fixture();
    other.name = "Other".into();
    method(&mut other, "equals", descriptor, 1, vec![0x04, 0xac]);
    let mut object = fixture();
    object.name = object_equals.owner.clone();
    object.super_name = None;
    method(&mut object, "equals", descriptor, 1, vec![]);
    object.methods[0].code = None;
    let mut integer = fixture();
    integer.name = "java/lang/Integer".into();
    method(&mut integer, "equals", descriptor, 1, vec![]);
    integer.methods[0].code = None;
    let mut classes = [class, other];
    let libraries = [object, integer];
    for expected in [true, false] {
        let program = application(&classes, &libraries);
        assert_eq!(
            program.canonical_boolean_return(None, InvokeKind::Virtual, &object_equals),
            expected,
        );
        assert!(program.canonical_boolean_return(Some(0), InvokeKind::Special, &object_equals));
        classes[1].methods[0].code.as_mut().unwrap().bytes = vec![0x05, 0xac];
    }
    // A supplied external override is not certified from its Z descriptor or
    // even an available body: it must have its own platform contract.
    let mut external = classes[1].clone();
    external.name = "External".into();
    external.methods[0].code.as_mut().unwrap().bytes = vec![0x04, 0xac];
    let classes = [classes[0].clone()];
    let libraries = [libraries[0].clone(), libraries[1].clone(), external];
    let program = application(&classes, &libraries);
    assert!(!program.canonical_boolean_return(None, InvokeKind::Virtual, &object_equals));
}

#[test]
fn special_returns_use_the_caller_superclass_selection_not_virtual_overrides() {
    let mut base = fixture();
    method(&mut base, "check", "()Z", 1, vec![0x04, 0xac]);
    let mut middle = fixture();
    middle.name = "Middle".into();
    middle.super_name = Some(base.name.clone());
    method(&mut middle, "check", "()Z", 1, vec![0x03, 0xac]);
    let mut child = fixture();
    child.name = "Child".into();
    child.access_flags |= 0x0020;
    child.super_name = Some(middle.name.clone());
    // The constant pool deliberately names the grandparent. ACC_SUPER still
    // selects Middle.check; Child.check is irrelevant to this exact call.
    let check = reference(&mut child, &base.name, "check", "()Z", false);
    let mut bytes = vec![0x2a];
    instruction(&mut bytes, 0xb7, check);
    bytes.push(0xac);
    method(&mut child, "caller", "()Z", 1 | 0x0010, bytes);
    method(&mut child, "check", "()Z", 1, vec![0x05, 0xac]);
    let mut classes = [base, middle, child];
    for expected in [true, false] {
        let program = application(&classes, &[]);
        assert_eq!(
            program.canonical_boolean_return(
                Some(2),
                InvokeKind::Special,
                &Member {
                    owner: "Flags".into(),
                    name: "check".into(),
                    descriptor: "()Z".into(),
                }
            ),
            expected,
        );
        assert_eq!(
            program.canonical_boolean_return(
                None,
                InvokeKind::Virtual,
                &Member {
                    owner: "Child".into(),
                    name: "caller".into(),
                    descriptor: "()Z".into(),
                }
            ),
            expected,
        );
        classes[1].methods[0].code.as_mut().unwrap().bytes = vec![0x05, 0xac];
    }
    // Legacy files without ACC_SUPER select the symbolic base itself.
    classes[2].access_flags &= !0x0020;
    let program = application(&classes, &[]);
    assert!(program.canonical_boolean_return(
        Some(2),
        InvokeKind::Special,
        &Member {
            owner: "Flags".into(),
            name: "check".into(),
            descriptor: "()Z".into(),
        }
    ));
}

#[test]
fn observed_public_arguments_flow_through_forwarders_and_virtual_implementations() {
    let mut base = fixture();
    base.access_flags |= 0x0400;
    field(&mut base, "saved", 1 | STATIC);
    let set = reference(&mut base, "Flags", "set", "(Z)V", false);
    method(&mut base, "set", "(Z)V", 1 | 0x0400, vec![]);
    base.methods[0].code = None;
    let forward = reference(&mut base, "Flags", "forward", "(LFlags;Z)V", false);
    let mut forwarding = vec![0x2a, 0x1b];
    instruction(&mut forwarding, 0xb6, set);
    forwarding.push(0xb1);
    let forwarding_member = method(&mut base, "forward", "(LFlags;Z)V", 1 | STATIC, forwarding);
    let mut caller = vec![0x2a, 0x04];
    instruction(&mut caller, 0xb8, forward);
    caller.push(0xb1);
    method(&mut base, "choose", "(LFlags;)V", 1 | STATIC, caller);
    let mut child = fixture();
    child.name = "Child".into();
    child.super_name = Some("Flags".into());
    let saved = reference(&mut child, "Flags", "saved", "Z", true);
    let mut setter = vec![0x1b];
    instruction(&mut setter, 0xb3, saved);
    setter.push(0xb1);
    method(&mut child, "set", "(Z)V", 1, setter);
    let mut other = child.clone();
    other.name = "Other".into();
    let mut classes = [base, child, other];
    {
        let program = application(&classes, &[]);
        assert!(certified(&program, "saved"));
        assert!(program.canonical_boolean_parameter(forwarding_member, 1));
        for class in [1, 2] {
            assert!(program.canonical_boolean_parameter(MemberId { class, index: 0 }, 0));
        }
    }
    let direct = reference(&mut classes[0], "Child", "set", "(Z)V", false);
    let mut raw = vec![0x2a, 0x05];
    instruction(&mut raw, 0xb6, direct);
    raw.push(0xb1);
    method(&mut classes[0], "raw", "(LChild;)V", 1 | STATIC, raw);
    let program = application(&classes, &[]);
    assert!(!certified(&program, "saved"));
    assert!(!program.canonical_boolean_parameter(MemberId { class: 1, index: 0 }, 0));
    assert!(program.canonical_boolean_parameter(MemberId { class: 2, index: 0 }, 0));
}

#[test]
fn api_callbacks_and_missing_providers_keep_inputs_open_despite_application_callers() {
    let mut base = fixture();
    let saved = field(&mut base, "saved", 1 | STATIC);
    let set = reference(&mut base, "Flags", "set", "(Z)V", false);
    let mut setter = vec![0x1b];
    instruction(&mut setter, 0xb3, saved);
    setter.push(0xb1);
    let target = method(&mut base, "set", "(Z)V", 1, setter);
    let mut caller = vec![0x2a, 0x04];
    instruction(&mut caller, 0xb6, set);
    caller.push(0xb1);
    method(&mut base, "choose", "(LFlags;)V", 1 | STATIC, caller);
    let mut child = fixture();
    child.name = "Child".into();
    child.super_name = Some("Flags".into());
    let mut api = fixture();
    api.name = "Listener".into();
    api.access_flags |= 0x0600;
    method(&mut api, "set", "(Z)V", 1 | 0x0400, vec![]);
    api.methods[0].code = None;
    let mut classes = [base, child];
    let libraries = [api];
    for (interfaces, expected) in [(vec![], true), (vec!["Listener".into()], false)] {
        // A descendant can adopt the callback without declaring another body.
        classes[1].interfaces = interfaces;
        let program = application(&classes, &libraries);
        assert_eq!(certified(&program, "saved"), expected);
        assert_eq!(program.canonical_boolean_parameter(target, 0), expected);
    }
    let program = application(&classes, &[]);
    assert!(!program.canonical_boolean_parameter(target, 0));
    assert!(!certified(&program, "saved"));
}

#[test]
fn partial_inputs_stay_open_after_observed_public_calls_and_writes() {
    let mut class = fixture();
    let saved = field(&mut class, "saved", 1 | STATIC);
    let private = field(&mut class, "private", PRIVATE | STATIC);
    let set = reference(&mut class, "Flags", "set", "(Z)V", false);
    let mut setter = vec![0x1a];
    instruction(&mut setter, 0xb3, saved);
    setter.push(0xb1);
    let target = method(&mut class, "set", "(Z)V", 1 | STATIC, setter);
    let mut caller = vec![0x04];
    instruction(&mut caller, 0xb8, set);
    caller.push(0x03);
    instruction(&mut caller, 0xb3, private);
    caller.push(0xb1);
    method(&mut class, "inside", "()V", 1 | STATIC, caller);
    let classes = [class];
    let partial = Program::new(&classes, &[]).unwrap();
    assert!(!partial.canonical_boolean_parameter(target, 0));
    assert!(!certified(&partial, "saved"));
    assert!(certified(&partial, "private"));
    let complete = application(&classes, &[]);
    assert!(complete.canonical_boolean_parameter(target, 0));
    assert!(certified(&complete, "saved"));
}

#[test]
fn partial_or_incomplete_virtual_families_do_not_close_after_one_known_body() {
    let mut base = fixture();
    method(&mut base, "check", "()Z", 1, vec![0x04, 0xac]);
    let check = Member {
        owner: "Flags".into(),
        name: "check".into(),
        descriptor: "()Z".into(),
    };
    let classes = [base.clone()];
    assert!(
        !Program::new(&classes, &[])
            .unwrap()
            .canonical_boolean_return(None, InvokeKind::Virtual, &check)
    );
    assert!(application(&classes, &[]).canonical_boolean_return(None, InvokeKind::Virtual, &check));
    let mut child = fixture();
    child.name = "Child".into();
    child.super_name = Some("Flags".into());
    child.interfaces.push("OmittedProvider".into());
    let classes = [base, child];
    assert!(!application(&classes, &[]).canonical_boolean_return(
        None,
        InvokeKind::Virtual,
        &check
    ));
}

#[test]
fn malformed_and_native_methods_preserve_independent_return_facts() {
    let mut intact = fixture();
    field(&mut intact, "privateDefault", PRIVATE | STATIC);
    method(&mut intact, "truth", "()Z", 1 | STATIC, vec![0x04, 0xac]);
    let mut broken = fixture();
    broken.name = "Broken".into();
    method(&mut broken, "malformed", "()V", 1 | STATIC, vec![0xff]);
    let classes = [intact.clone(), broken.clone()];
    assert!(certified(&application(&classes, &[]), "privateDefault"));
    method(&mut broken, "opaque", "()V", 1 | STATIC | NATIVE, vec![]);
    broken.methods[1].code = None;
    let classes = [intact, broken];
    let program = application(&classes, &[]);
    assert!(!certified(&program, "privateDefault"));
    assert!(program.canonical_boolean_return(
        None,
        InvokeKind::Static,
        &Member {
            owner: "Flags".into(),
            name: "truth".into(),
            descriptor: "()Z".into()
        }
    ));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn compact_summaries_match_serial_and_parallel_preparation() {
    let mut class = fixture();
    let saved = field(&mut class, "saved", PRIVATE | STATIC);
    let pass = reference(&mut class, "Flags", "pass", "(Z)Z", false);
    method(
        &mut class,
        "pass",
        "(Z)Z",
        PRIVATE | STATIC,
        vec![0x1a, 0xac],
    );
    let mut caller = vec![0x04];
    instruction(&mut caller, 0xb8, pass);
    instruction(&mut caller, 0xb3, saved);
    caller.push(0xb1);
    method(&mut class, "inside", "()V", 1 | STATIC, caller);
    let classes = [class];
    let expected = Summary::build(&application(&classes, &[]));
    assert!(!expected.canonical.is_empty());
    for threads in [1, 2, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        let program = application(&classes, &[]);
        assert_eq!(pool.install(|| Summary::build_parallel(&program)), expected);
        let mut prepared = application(&classes, &[]);
        pool.install(|| prepared.prepare_analysis());
        assert!(certified(&prepared, "saved"));
    }
}
