use super::*;
use crate::classfile::{ClassFile, Code, ExceptionHandler, Method};
use crate::ir::ValueUse;
use crate::java::{expr::Helper, special::SpecialPlan};
use crate::naming::NamePlan;

fn fixture() -> ClassFile {
    let mut class = ClassFile::parse(include_bytes!(
        "../fixtures/legacy/classes/pkg/TestConsecutiveReads.class"
    ))
    .unwrap();
    class.methods = vec![Method {
        name: "run".into(),
        descriptor: "()V".into(),
        access_flags: 9,
        exceptions: Vec::new(),
        synthetic: false,
        code: Some(Code {
            max_stack: 1,
            max_locals: 1,
            // A self-protected catch/rethrow forces explicit dispatch.
            bytes: vec![0x01, 0xbf, 0x4b, 0x2a, 0xbf],
            exception_table: vec![ExceptionHandler {
                start_pc: 0,
                end_pc: 5,
                handler_pc: 2,
                catch_type: None,
            }],
            line_numbers: Vec::new(),
            local_variables: Vec::new(),
        }),
    }];
    class.fields.clear();
    class
}

#[test]
fn allocation_artifact_retains_dispatch_recovery_from_its_helper() {
    let classes = [fixture()];
    let program = Program::new(&classes, &[]).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let types = TypeNames::with_methods(&program, &names, 0, [], &[], None);
    let cx = EmissionContext::new(&program, &types, SpecialPlan::default());
    let member = MemberId { class: 0, index: 0 };
    let ir = crate::ir::decompile_method(&classes[0], &classes[0].methods[0]).unwrap();
    let helpers = AllocationHelpers(vec![AllocationHelper {
        header: "private static void extracted() {".into(),
        ir: PreparedIr::new(ir),
        carrier: String::new(),
    }]);
    let rendered = helpers.render(&cx, member).unwrap();
    assert!(rendered.fallback, "{}", rendered.text);
    assert!(rendered.text.contains("switch"), "{}", rendered.text);
}

#[test]
fn allocation_extraction_ignores_shadowed_inner_handlers() {
    use crate::classfile::Constant;
    let mut class = fixture();
    class.name = "Probe".into();
    class.major_version = 49;
    class.constant_pool = Default::default();
    class.constant_pool.entries = vec![
        Constant::Unusable,
        Constant::Utf8("External".into()),
        Constant::Class(1),
        Constant::Utf8("Probe".into()),
        Constant::Class(3),
        Constant::Utf8("tick".into()),
        Constant::Utf8("()I".into()),
        Constant::NameAndType {
            name_index: 5,
            descriptor_index: 6,
        },
        Constant::Methodref {
            class_index: 4,
            name_and_type_index: 7,
        },
        Constant::Utf8("<init>".into()),
        Constant::Utf8("(I)V".into()),
        Constant::NameAndType {
            name_index: 9,
            descriptor_index: 10,
        },
        Constant::Methodref {
            class_index: 2,
            name_and_type_index: 11,
        },
    ];
    let method = &mut class.methods[0];
    method.descriptor = "()LExternal;".into();
    let code = method.code.as_mut().unwrap();
    code.max_stack = 2;
    code.max_locals = 3;
    // Save NEW, evaluate tick under a catch-all, then initialize with its
    // result. The second catch has a different result but can never execute.
    code.bytes = vec![
        0xbb, 0, 2, 0x4c, 0xb8, 0, 8, 0x3b, 0xa7, 0, 15, 0x4d, 0x04, 0x3b, 0xa7, 0, 9, 0x4d, 0x05,
        0x3b, 0xa7, 0, 3, 0x2b, 0x1a, 0xb7, 0, 12, 0x2b, 0xb0,
    ];
    code.exception_table = vec![ExceptionHandler {
        start_pc: 4,
        end_pc: 8,
        handler_pc: 11,
        catch_type: None,
    }];
    class.methods.push(Method {
        name: "tick".into(),
        descriptor: "()I".into(),
        access_flags: 0x109,
        code: None,
        exceptions: vec![],
        synthetic: false,
    });
    let render = |class: &ClassFile| {
        let program = Program::new(std::slice::from_ref(class), &[]).unwrap();
        let names = NamePlan::build(&program, &Default::default()).unwrap();
        let output =
            crate::java::decompile_class(&program, &names, 0, &Default::default()).unwrap();
        assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
        output.source
    };
    let expected = render(&class);
    class.methods[0]
        .code
        .as_mut()
        .unwrap()
        .exception_table
        .push(ExceptionHandler {
            start_pc: 4,
            end_pc: 8,
            handler_pc: 17,
            catch_type: Some("java/lang/Exception".into()),
        });
    assert_eq!(render(&class), expected);
}

#[test]
fn allocation_tail_facts_preserve_branch_and_exceptional_assignment_boundaries() {
    use crate::index::test_ids::*;
    use crate::ir::{CompareOp, Condition, Literal};
    let condition = Condition {
        op: CompareOp::Eq,
        left: V0,
        right: V4,
    };
    let mut ir = MethodIr {
        entry: B0,
        values: [
            ValueKind::Parameter {
                slot: 0,
                index: Some(0),
            },
            ValueKind::Phi {
                inputs: vec![],
                slot: None,
            },
            ValueKind::Phi {
                inputs: vec![],
                slot: None,
            },
            ValueKind::Phi {
                inputs: vec![],
                slot: None,
            },
            ValueKind::Literal(Literal::Int(0)),
            ValueKind::Call {
                kind: InvokeKind::Static,
                method: Member {
                    owner: "Effects".into(),
                    name: "run".into(),
                    descriptor: "()I".into(),
                },
                receiver: None,
                args: vec![],
            },
            ValueKind::InlineAssign {
                local: L3,
                value: V4,
            },
            ValueKind::Select {
                condition: condition.clone(),
                then_value: V6,
                else_value: V4,
            },
        ]
        .into_iter()
        .enumerate()
        .map(|(index, kind)| Value {
            local: (index < 4).then_some(LocalId::from_index(index)),
            kind,
            ty: JvmType::Int,
            pc: 0,
            block: B0,
        })
        .collect(),
        blocks: (0..5)
            .map(|_| IrBlock {
                caught_exception: None,
                pc: 0,
                statements: vec![],
                terminator: Terminator::Goto(B4),
                handlers: vec![],
            })
            .collect(),
        locals: (0..4)
            .map(|index| Local {
                name_hint: None,
                name: format!("v{index}"),
                ty: JvmType::Int,
                parameter: None,
                is_this: false,
                needs_default: false,
            })
            .collect(),
        parameters: vec![V0],
        return_type: JvmType::Int,
        initialization_sites: vec![],
    };
    ir.blocks[B0].terminator = Terminator::Branch {
        condition,
        then_target: B1,
        else_target: B2,
    };
    ir.blocks[B1].statements.push(Statement::Assign {
        local: L1,
        value: ValueUse::Reference(V5),
    });
    ir.blocks[B2].statements.push(Statement::Assign {
        local: L1,
        value: ValueUse::Reference(V4),
    });
    for block in [B1, B2, B3] {
        ir.blocks[block].statements.push(Statement::Assign {
            local: L2,
            value: ValueUse::Reference(V4),
        });
    }
    ir.blocks[B4].terminator = Terminator::Return(None);
    let shape = graph::Shape {
        entry: B0,
        end: B4,
        allocation_pc: 0,
        occurrence: 0,
        spans: ir
            .blocks
            .iter_enumerated()
            .map(|(block, body)| (block, 0..body.statements.len()))
            .collect(),
        handlers: BTreeMap::from([(
            B1,
            vec![crate::cfg::HandlerEdge {
                target: B3,
                catch_type: None,
                table_index: 0,
            }],
        )]),
    };
    let facts = graph::facts(&ir, &shape);
    assert_eq!(facts.inputs, BTreeSet::from([L0]));
    assert_eq!(facts.definite, BTreeSet::from([L2]));
    // Both normal arms assign L1, but its throwing producer can enter the catch
    // before committing. The tail must still capture its incoming value.
    assert_eq!(
        facts.with_arguments(&ir, &[V1, V2]).inputs,
        BTreeSet::from([L0, L1])
    );
    let conditional = facts.with_arguments(&ir, &[V7, V3]);
    assert_eq!(conditional.inputs, BTreeSet::from([L0, L3]));
    assert!(conditional.written.contains(&L3));
    assert!(!conditional.definite.contains(&L3));
    // Argument order matters: a definite capture supplies the following read.
    let assigned = facts.with_arguments(&ir, &[V6, V3]);
    assert_eq!(assigned.inputs, BTreeSet::from([L0]));
    assert_eq!(assigned.definite, BTreeSet::from([L2, L3]));
    assert_eq!(
        facts.with_arguments(&ir, &[V3, V6]).inputs,
        BTreeSet::from([L0, L3])
    );
    assert_eq!(facts.written, BTreeSet::from([L1, L2]));

    // A graph edit needs a new summary: now the catch also commits L1.
    ir.blocks[B3].statements[0] = Statement::Assign {
        local: L1,
        value: ValueUse::Reference(V4),
    };
    let revised = graph::facts(&ir, &shape).with_arguments(&ir, &[V1, V2]);
    assert_eq!(revised.inputs, BTreeSet::from([L0, L2]));
    assert_eq!(revised.definite, BTreeSet::from([L1]));
}

#[test]
fn failed_body_candidate_cannot_publish_support_or_generated_contracts() {
    let classes = [fixture()];
    let program = Program::new(&classes, &[]).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let types = TypeNames::with_methods(&program, &names, 0, [], &[], None);
    let cx = EmissionContext::new(&program, &types, SpecialPlan::default());
    let member = Member {
        owner: classes[0].name.clone(),
        name: "$candidate".into(),
        descriptor: "()V".into(),
    };
    let failed: Result<super::super::context::Artifact<()>> = cx.artifact(|candidate| {
        candidate.helper(Helper::CompareLong);
        candidate.register(&member, Default::default())?;
        anyhow::bail!("unrepresentable candidate")
    });
    assert!(failed.is_err());
    let mut output = CodeWriter::new(1);
    cx.emit_support(&mut output);
    assert!(output.text.is_empty());
    assert!(
        cx.generated
            .borrow()
            .get(&member.owner, &member.name, &member.descriptor)
            .is_none()
    );
    let accepted = cx
        .artifact(|candidate| {
            candidate.helper(Helper::CompareLong);
            candidate.register(&member, Default::default())
        })
        .unwrap();
    assert!(
        cx.generated
            .borrow()
            .get(&member.owner, &member.name, &member.descriptor)
            .is_none()
    );
    cx.commit(accepted);
    cx.emit_support(&mut output);
    assert!(output.text.contains("$sf$cmpLong"));
    assert!(
        cx.generated
            .borrow()
            .get(&member.owner, &member.name, &member.descriptor)
            .is_some()
    );
}

#[test]
fn helper_symbols_reserve_mapped_members_and_nested_binary_names() {
    let mut nested = fixture();
    nested.name = "Other".into();
    let classes = [fixture(), nested];
    let program = Program::new(&classes, &[]).unwrap();
    let mut overrides = crate::naming::NameOverrides::default();
    overrides.classes.insert(0, "mapped/Owner".into());
    overrides.classes.insert(1, "mapped/Owner$$Preinit0".into());
    overrides
        .methods
        .insert(MemberId { class: 0, index: 0 }, "$preinit0".into());
    let names = NamePlan::build_with_overrides(&program, &Default::default(), &overrides).unwrap();
    let types = TypeNames::with_methods(&program, &names, 0, [], &[], None);
    assert_eq!(
        super::super::generated::symbol(&program, &types, "$Preinit0"),
        "$Preinit0_"
    );
    assert_eq!(
        super::super::generated::symbol(&program, &types, "$preinit0"),
        "$preinit0_"
    );
    assert_eq!(
        super::super::generated::symbol(&program, &types, "run"),
        "run_"
    );
}
