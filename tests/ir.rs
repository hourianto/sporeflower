use sporeflower::classfile::{ClassFile, Code, Constant, ConstantPool, JvmType, Method};
use sporeflower::index::{IdVec, LocalId, ValueId};
use sporeflower::ir::{
    self, BinaryOp, CompareOp, Literal, MethodIr, Statement, Terminator, ValueKind,
};

fn fixture(bytes: &[u8], descriptor: &str, max_stack: u16, max_locals: u16) -> ClassFile {
    ClassFile {
        minor_version: 0,
        major_version: 48,
        access_flags: 0x21,
        name: "Probe".into(),
        super_name: Some("java/lang/Object".into()),
        interfaces: vec![],
        constant_pool: ConstantPool::default(),
        fields: vec![],
        methods: vec![Method {
            access_flags: 9,
            name: "run".into(),
            descriptor: descriptor.into(),
            code: Some(Code {
                max_stack,
                max_locals,
                bytes: bytes.to_vec(),
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

fn lift(class: &ClassFile) -> MethodIr {
    ir::decompile_method(class, &class.methods[0]).unwrap()
}

// This interpreter executes the scheduled, register-based output. It has no JVM
// operand stack or local slots; wrong SSA joins and expression copies therefore
// fail independently of the bytecode interpreter inside the lifter.
fn execute(ir: &MethodIr, args: &[i32]) -> i32 {
    fn value(
        ir: &MethodIr,
        locals: &mut IdVec<LocalId, i32>,
        id: ValueId,
        definition: bool,
    ) -> i32 {
        if !definition && let Some(local) = ir.values[id].local {
            return locals[local];
        }
        let mut read = |id| value(ir, locals, id, false);
        match &ir.values[id].kind {
            ValueKind::Literal(Literal::Int(number)) => *number,
            ValueKind::Alias(id) => read(*id),
            ValueKind::LocalUpdate {
                local,
                increment,
                postfix,
                ..
            } => {
                // Execute the source storage operation, independently of the
                // original arithmetic graph retained for IR analysis.
                let old = locals[*local];
                let next = old.wrapping_add(if *increment { 1 } else { -1 });
                locals[*local] = next;
                if *postfix { old } else { next }
            }
            ValueKind::Select {
                condition,
                then_value,
                else_value,
            } => {
                let a = read(condition.left);
                let b = read(condition.right);
                let yes = match condition.op {
                    CompareOp::Eq => a == b,
                    CompareOp::Ne => a != b,
                    CompareOp::Lt => a < b,
                    CompareOp::Le => a <= b,
                    CompareOp::Gt => a > b,
                    CompareOp::Ge => a >= b,
                };
                read(if yes { *then_value } else { *else_value })
            }
            ValueKind::Binary { op, left, right } => {
                let a = read(*left);
                let b = read(*right);
                match op {
                    BinaryOp::Add => a.wrapping_add(b),
                    BinaryOp::Sub => a.wrapping_sub(b),
                    BinaryOp::Mul => a.wrapping_mul(b),
                    BinaryOp::Div => a.wrapping_div(b),
                    BinaryOp::Rem => a.wrapping_rem(b),
                    BinaryOp::And => a & b,
                    BinaryOp::Or => a | b,
                    BinaryOp::Xor => a ^ b,
                    BinaryOp::Shl => a.wrapping_shl(b as u32),
                    BinaryOp::Shr => a.wrapping_shr(b as u32),
                    BinaryOp::Ushr => (a as u32).wrapping_shr(b as u32) as i32,
                }
            }
            other => panic!("unexpected output expression {other:?}"),
        }
    }
    let mut locals: IdVec<LocalId, _> = vec![0; ir.locals.len()].into();
    for (id, local) in ir.locals.iter_enumerated() {
        if let Some(parameter) = local.parameter {
            locals[id] = args[parameter];
        }
    }
    let mut block = ir.entry;
    for _ in 0..10000 {
        for statement in &ir.blocks[block].statements {
            match statement {
                Statement::Assign { local, value: id } => {
                    locals[*local] = value(
                        ir,
                        &mut locals,
                        id.value(),
                        matches!(id, ir::ValueUse::Definition(_)),
                    )
                }
                other => panic!("unexpected output statement {other:?}"),
            }
        }
        match &ir.blocks[block].terminator {
            Terminator::Goto(next) => block = *next,
            Terminator::Return(Some(id)) => return value(ir, &mut locals, *id, false),
            Terminator::Branch {
                condition,
                then_target,
                else_target,
            } => {
                let a = value(ir, &mut locals, condition.left, false);
                let b = value(ir, &mut locals, condition.right, false);
                let yes = match condition.op {
                    CompareOp::Eq => a == b,
                    CompareOp::Ne => a != b,
                    CompareOp::Lt => a < b,
                    CompareOp::Le => a <= b,
                    CompareOp::Gt => a > b,
                    CompareOp::Ge => a >= b,
                };
                block = if yes { *then_target } else { *else_target };
            }
            other => panic!("unexpected output terminator {other:?}"),
        }
    }
    panic!("scheduled program did not terminate")
}

#[test]
fn saved_stack_value_survives_local_overwrite() {
    // iload_0; dup; iconst_1; iadd; istore_0; iload_0; iadd; ireturn
    let ir = lift(&fixture(
        &[0x1a, 0x59, 0x04, 0x60, 0x3b, 0x1a, 0x60, 0xac],
        "(I)I",
        3,
        1,
    ));
    for input in [i32::MIN, -1, 0, 9, i32::MAX] {
        assert_eq!(
            execute(&ir, &[input]),
            input.wrapping_add(input.wrapping_add(1))
        );
    }
}

#[test]
fn diamond_merges_stack_values_without_losing_branch_assignment() {
    let ir = lift(&fixture(
        &[0x1a, 0x99, 0, 7, 0x04, 0xa7, 0, 4, 0x05, 0xac],
        "(I)I",
        1,
        1,
    ));
    assert_eq!(execute(&ir, &[0]), 2);
    assert_eq!(execute(&ir, &[7]), 1);
}

#[test]
fn shared_handler_stack_joins_do_not_depend_on_edge_discovery_order() {
    // Both layouts merge a normal null with the exception produced by
    // arraylength(null). The second layout visits the protected branch first.
    for (bytes, start, end) in [
        (
            vec![
                0x1a, 0x99, 0, 7, 0x01, 0xa7, 0, 7, 0x01, 0xbe, 0x57, 0x01, 0x4c, 0x2b, 0xb0,
            ],
            8,
            10,
        ),
        (
            vec![
                0x1a, 0x99, 0, 10, 0x01, 0xbe, 0x57, 0x01, 0xa7, 0, 4, 0x01, 0x4c, 0x2b, 0xb0,
            ],
            4,
            6,
        ),
    ] {
        let mut class = fixture(&bytes, "(I)Ljava/lang/Object;", 1, 2);
        class.methods[0]
            .code
            .as_mut()
            .unwrap()
            .exception_table
            .push(sporeflower::classfile::ExceptionHandler {
                start_pc: start,
                end_pc: end,
                handler_pc: 12,
                catch_type: None,
            });
        let ir = lift(&class);
        let (handler, caught) = ir
            .blocks
            .iter_enumerated()
            .find_map(|(block, value)| value.caught_exception.map(|value| (block, value)))
            .unwrap();
        let returned = match ir.blocks[handler].terminator {
            Terminator::Return(Some(value)) => value,
            _ => panic!("shared entry lost its return"),
        };
        assert!(
            matches!(&ir.values[returned].kind, ValueKind::Phi { inputs, slot: None } if inputs.iter().any(|input| input.throw_pc.is_none()) && inputs.iter().any(|input| input.throw_pc.is_some()))
        );
        assert_eq!(ir.values[caught].local, ir.values[returned].local);
        for (block, data) in ir.blocks.iter_enumerated() {
            if block != handler {
                assert!(
                    !data
                        .statements
                        .iter()
                        .any(|statement| statement.operands().contains(&caught)),
                    "exception value captured before the catch edge"
                );
            }
        }
    }
}

#[test]
fn loop_phi_copies_execute_on_the_taken_backedge() {
    // int sum=0, i=0; while (i<n) {sum+=i; ++i;} return sum;
    let code = [
        0x03, 0x3c, 0x03, 0x3d, 0x1c, 0x1a, 0xa2, 0, 13, 0x1b, 0x1c, 0x60, 0x3c, 0x84, 2, 1, 0xa7,
        0xff, 0xf4, 0x1b, 0xac,
    ];
    let ir = lift(&fixture(&code, "(I)I", 2, 3));
    for input in [-1, 0, 1, 2, 9, 100] {
        assert_eq!(execute(&ir, &[input]), (0..input).sum::<i32>());
    }
}

#[test]
fn parallel_phi_copies_capture_inline_dependencies_before_overwrite() {
    // int a=0,b=1; while(n-- > 0){int sum=a+b; a=b; b=sum;} return a;
    let code = [
        0x03, 0x3c, 0x04, 0x3d, 0x1a, 0x84, 0, 0xff, 0x9e, 0, 14, 0x1b, 0x1c, 0x60, 0x3e, 0x1c,
        0x3c, 0x1d, 0x3d, 0xa7, 0xff, 0xf1, 0x1b, 0xac,
    ];
    let ir = lift(&fixture(&code, "(I)I", 2, 4));
    for (n, expected) in [0, 1, 1, 2, 3, 5, 8, 13, 21, 34].into_iter().enumerate() {
        assert_eq!(execute(&ir, &[n as i32]), expected, "fib({n})");
    }
}

#[test]
fn stack_phi_capture_keeps_a_loop_cursor_in_one_source_local() {
    // cursor=0; sum=0; for(i=0;i<count;i++)
    //   sum += (i&1)==0 ? cursor++ : input*2+1;
    // The old cursor/index is an operand-stack join, while JVM slot 2 keeps
    // the updated cursor. Its use must precede the cursor update, whether the
    // index is captured or its addition is distributed into the predecessors.
    let code = [
        0x03, 0x3d, 0x03, 0x3e, 0x03, 0x36, 0x04, 0x15, 0x04, 0x1b, 0xa2, 0x00, 0x1f, 0x15, 0x04,
        0x04, 0x7e, 0x9a, 0x00, 0x0a, 0x1c, 0x84, 0x02, 0x01, 0xa7, 0x00, 0x08, 0x1a, 0x05, 0x68,
        0x04, 0x60, 0x1d, 0x60, 0x3e, 0x84, 0x04, 0x01, 0xa7, 0xff, 0xe1, 0x1c, 0x10, 0x64, 0x68,
        0x1d, 0x60, 0xac,
    ];
    let ir = lift(&fixture(&code, "(II)I", 3, 5));
    let cursor_locals: std::collections::BTreeSet<_> = ir
        .values
        .iter()
        .filter_map(|value| {
            matches!(value.kind, ValueKind::Phi { slot: Some(2), .. })
                .then_some(value.local)
                .flatten()
        })
        .collect();
    assert_eq!(
        cursor_locals.len(),
        1,
        "the cursor must not be saved and restored: {ir:#?}"
    );
    assert!(
        ir.values
            .iter()
            .filter_map(|value| {
                matches!(value.kind, ValueKind::Phi { slot: None, .. })
                    .then_some(value.local)
                    .flatten()
            })
            .all(|local| !cursor_locals.contains(&local)),
        "a transient stack result must not overwrite the cursor"
    );
    for input in [-3, 0, 7] {
        for count in [-1, 0, 1, 2, 11] {
            let mut cursor = 0;
            let mut sum = 0;
            for iteration in 0..count {
                sum += if iteration & 1 == 0 {
                    let old = cursor;
                    cursor += 1;
                    old
                } else {
                    input * 2 + 1
                };
            }
            assert_eq!(execute(&ir, &[input, count]), cursor * 100 + sum);
        }
    }
}

#[test]
fn category_two_duplication_preserves_one_value_identity() {
    let ir = lift(&fixture(
        &[0x1e, 0x5c, 0x0a, 0x61, 0x3f, 0xad],
        "(J)J",
        6,
        2,
    ));
    let returned = ir
        .blocks
        .iter()
        .find_map(|block| {
            if let Terminator::Return(value) = block.terminator {
                value
            } else {
                None
            }
        })
        .unwrap();
    assert!(matches!(
        ir.values[returned].kind,
        ValueKind::Parameter { slot: 0, .. }
    ));
    assert_eq!(ir.values[returned].ty, JvmType::Long);
}

#[test]
fn equal_calls_are_distinct_but_dup_is_shared() {
    let mut class = fixture(
        &[0xb8, 0, 6, 0x59, 0xb8, 0, 6, 0x60, 0x60, 0xac],
        "()I",
        3,
        0,
    );
    class.constant_pool.entries = vec![
        Constant::Unusable,
        Constant::Utf8("Probe".into()),
        Constant::Class(1),
        Constant::Utf8("read".into()),
        Constant::Utf8("()I".into()),
        Constant::NameAndType {
            name_index: 3,
            descriptor_index: 4,
        },
        Constant::Methodref {
            class_index: 2,
            name_and_type_index: 5,
        },
    ];
    let ir = lift(&class);
    let calls: Vec<_> = ir
        .values
        .iter()
        .filter(|value| matches!(value.kind, ValueKind::Call { .. }))
        .collect();
    assert_eq!(calls.len(), 2);
    assert!(
        calls[0].local.is_some(),
        "a duplicated observable call requires one captured value"
    );
}

#[test]
fn overwritten_receiver_slot_is_a_value_not_this_assignment() {
    let mut class = fixture(&[0x01, 0x4b, 0x2a, 0xb0], "()Ljava/lang/Object;", 1, 1);
    class.methods[0].access_flags = 1;
    let ir = lift(&class);
    assert!(ir.locals.iter().any(|local| local.is_this));
    for block in &ir.blocks {
        for statement in &block.statements {
            if let Statement::Assign { local, .. } = statement {
                assert!(!ir.locals[*local].is_this);
            }
        }
    }
}

#[test]
fn boolean_array_load_tracks_its_phi_after_null_first_predecessor() {
    let code = [
        0x01, 0x4c, 0x1a, 0x99, 0, 11, 0x04, 0xbc, 4, 0x4c, 0x2b, 0x03, 0x04, 0x54, 0x2b, 0x03,
        0x33, 0xac,
    ];
    let ir = lift(&fixture(&code, "(Z)Z", 3, 2));
    let load = ir
        .values
        .iter()
        .find(|value| matches!(value.kind, ValueKind::ArrayLoad { .. }))
        .unwrap();
    assert_eq!(load.ty, JvmType::Boolean);
    assert!(matches!(
        load.kind,
        ValueKind::ArrayLoad {
            element_type: JvmType::Boolean,
            ..
        }
    ));
}

#[test]
fn reference_joins_use_known_widening_and_keep_unknown_types_conservative() {
    let code = [0x1c, 0x99, 0, 7, 0x2a, 0xa7, 0, 4, 0x2b, 0xb0];
    let class = fixture(&code, "(LChild;LBase;Z)LBase;", 1, 3);
    let child = JvmType::Object("Child".into());
    let base = JvmType::Object("Base".into());
    let known = |from: &JvmType, to: &JvmType| from == to || from == &child && to == &base;
    let unknown = |_: &JvmType, _: &JvmType| false;
    for (fact, expected) in [
        (None, JvmType::Object("java/lang/Object".into())),
        (
            Some(&unknown as &dyn Fn(&JvmType, &JvmType) -> bool),
            JvmType::Object("java/lang/Object".into()),
        ),
        (
            Some(&known as &dyn Fn(&JvmType, &JvmType) -> bool),
            base.clone(),
        ),
    ] {
        let ir = ir::decompile_method_with_context(
            &class,
            &class.methods[0],
            &ir::MethodContext {
                reference_assignable: fact,
                ..Default::default()
            },
        )
        .unwrap();
        let result = ir
            .blocks
            .iter()
            .find_map(|block| match block.terminator {
                Terminator::Return(Some(value)) => Some(value),
                _ => None,
            })
            .unwrap();
        assert_eq!(ir.values[result].ty, expected);
    }
}

#[test]
fn sparse_wide_slots_do_not_allocate_phis_for_unused_slot_numbers() {
    let code = [
        0x04, 0xc4, 0x36, 0xff, 0xfe, 0xa7, 0, 3, 0xc4, 0x15, 0xff, 0xfe, 0xac,
    ];
    let ir = lift(&fixture(&code, "()I", 1, u16::MAX));
    assert_eq!(execute(&ir, &[]), 1);
    assert!(ir.values.len() < 10);
}

#[test]
fn handler_liveness_keeps_each_prethrow_local_definition() {
    let code = [
        0x1a, 0x3c, 0x04, 0x1a, 0x6c, 0x57, 0x10, 7, 0x3c, 0x04, 0x03, 0x6c, 0xac, 0x4d, 0x1b, 0xac,
    ];
    let mut class = fixture(&code, "(I)I", 2, 3);
    class.methods[0]
        .code
        .as_mut()
        .unwrap()
        .exception_table
        .push(sporeflower::classfile::ExceptionHandler {
            start_pc: 2,
            end_pc: 13,
            handler_pc: 13,
            catch_type: Some("java/lang/ArithmeticException".into()),
        });
    let ir = lift(&class);
    let inputs = ir
        .values
        .iter()
        .find_map(|value| match &value.kind {
            ValueKind::Phi {
                slot: Some(1),
                inputs,
            } if inputs.iter().any(|input| input.throw_pc.is_some()) => Some(inputs),
            _ => None,
        })
        .unwrap();
    let first = inputs
        .iter()
        .find(|input| input.throw_pc == Some(4))
        .unwrap();
    let second = inputs
        .iter()
        .find(|input| input.throw_pc == Some(11))
        .unwrap();
    assert!(matches!(
        ir.values[first.value].kind,
        ValueKind::Parameter { index: Some(0), .. }
    ));
    assert!(matches!(
        ir.values[second.value].kind,
        ValueKind::Literal(Literal::Int(7))
    ));
}

#[test]
fn shadowed_handlers_do_not_keep_local_snapshots_alive() {
    // The second catch reads slot 1 at both division sites, but its repeated
    // type means neither exceptional edge can execute. That read used to keep
    // the initial parameter copy and the later store of 7 alive in scheduled IR.
    let bytes = [
        0x1a, 0x3c, 0x04, 0x1a, 0x6c, 0x57, 0x10, 7, 0x3c, 0x04, 0x03, 0x6c, 0xac, 0x57, 0x02,
        0xac, 0x4d, 0x1b, 0xac,
    ];
    let mut class = fixture(&bytes, "(I)I", 2, 3);
    let table = &mut class.methods[0].code.as_mut().unwrap().exception_table;
    for handler_pc in [13, 16] {
        table.push(sporeflower::classfile::ExceptionHandler {
            start_pc: 2,
            end_pc: 13,
            handler_pc,
            catch_type: Some("java/lang/ArithmeticException".into()),
        });
    }
    let ir = lift(&class);
    let protected = ir.blocks.iter().find(|block| block.pc == 2).unwrap();
    assert_eq!(protected.handlers.len(), 2, "lexical table was erased");
    let shadowed = &ir.blocks[protected.handlers[1].target];
    assert!(matches!(shadowed.terminator, Terminator::Unreachable));
    assert!(shadowed.caught_exception.is_none());
    let classes = vec![class];
    let program = sporeflower::program::Program::new(&classes, &[]).unwrap();
    let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
    let output =
        sporeflower::java::decompile_class(&program, &names, 0, &Default::default()).unwrap();
    assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
    assert!(!output.source.contains("= 7;"), "{}", output.source);
    assert_eq!(
        output.source.matches("catch (").count(),
        1,
        "{}",
        output.source
    );
    assert!(
        !ir.values.iter().any(|value| matches!(
            &value.kind,
            ValueKind::Phi { slot: Some(1), inputs } if !inputs.is_empty()
        )),
        "shadowed handler retained local snapshots"
    );
}

#[test]
fn shared_handler_type_includes_its_catch_all_entry() {
    let mut class = fixture(&[0x04, 0x1a, 0x6c, 0xac, 0x4c, 0x2b, 0xbf], "(I)I", 2, 2);
    let exceptions = &mut class.methods[0].code.as_mut().unwrap().exception_table;
    for catch_type in [Some("java/lang/NullPointerException".into()), None] {
        exceptions.push(sporeflower::classfile::ExceptionHandler {
            start_pc: 0,
            end_pc: 4,
            handler_pc: 4,
            catch_type,
        });
    }
    let ir = lift(&class);
    let caught = ir
        .blocks
        .iter()
        .find_map(|block| block.caught_exception)
        .unwrap();
    assert_eq!(
        ir.values[caught].ty,
        JvmType::Object("java/lang/Throwable".into())
    );
}
