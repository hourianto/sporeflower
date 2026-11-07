use super::*;
use crate::{
    bytecode::decode,
    cfg::PreparedCode,
    classfile::{Code, ConstantPool},
    ir::{self, Terminator, ValueKind},
};

fn fixture(bytes: &[u8], descriptor: &str, max_stack: u16, max_locals: u16) -> ClassFile {
    ClassFile {
        minor_version: 0,
        major_version: 48,
        access_flags: 0x21,
        name: "FrameProbe".into(),
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

#[test]
fn wide_local_and_category_two_permutation_keep_the_same_throw_operand_in_both_domains() {
    // aload_0; lload_1; dup2_x1; pop2; wide astore 300; pop2;
    // wide aload 300; athrow. The saved reference crosses both copies of a long.
    let class = fixture(
        &[
            0x2a, 0x1f, 0x5d, 0x58, 0xc4, 0x3a, 0x01, 0x2c, 0x58, 0xc4, 0x19, 0x01, 0x2c, 0xbf,
        ],
        "(Ljava/io/IOException;J)V",
        5,
        301,
    );
    let program = Program::new(std::slice::from_ref(&class), &[]).unwrap();
    let method = &class.methods[0];
    let prepared = PreparedCode::new(method.code.as_ref().unwrap()).unwrap();
    let summary = analyze(
        &program,
        &class,
        method,
        &prepared.instructions,
        &prepared.graph,
    )
    .unwrap();
    let thrown = &summary.throws[&13];
    assert_eq!(thrown.source_type, "java/io/IOException");
    assert_eq!(
        thrown.origins,
        BTreeSet::from([Origin::Direct(Arc::from("java/io/IOException"))])
    );

    let ir = ir::decompile_method(&class, method).unwrap();
    let value = ir
        .blocks
        .iter()
        .find_map(|block| match block.terminator {
            Terminator::Throw { value, .. } => Some(value),
            _ => None,
        })
        .unwrap();
    assert!(matches!(
        ir.values[value].kind,
        ValueKind::Parameter { slot: 0, .. }
    ));
    assert_eq!(
        ir.values[value].ty,
        JvmType::Object("java/io/IOException".into())
    );
}

#[test]
fn numeric_operations_consume_operands_in_reference_frames_too() {
    let class = fixture(&[0x74, 0x01, 0xbf], "()V", 1, 0);
    let program = Program::new(std::slice::from_ref(&class), &[]).unwrap();
    let method = &class.methods[0];
    let prepared = PreparedCode::new(method.code.as_ref().unwrap()).unwrap();
    let error = analyze(
        &program,
        &class,
        method,
        &prepared.instructions,
        &prepared.graph,
    )
    .err()
    .unwrap();
    assert!(format!("{error:#}").contains("operand stack underflow"));
    assert!(ir::decompile_method(&class, method).is_err());
}

#[test]
fn conversions_preserve_category_two_stack_shape_before_athrow() {
    // Every conversion result is popped with its JVM category before throwing
    // the untouched reference below it. No Java execution or source renderer is involved.
    for (opcode, input, discard) in [
        (0x85, 0x03, 0x58), // i2l
        (0x88, 0x09, 0x57), // l2i
        (0x8c, 0x0b, 0x58), // f2l
        (0x8f, 0x0e, 0x58), // d2l
        (0x90, 0x0e, 0x57), // d2f
    ] {
        let bytes = [0x2a, input, opcode, discard, 0xbf];
        let class = fixture(&bytes, "(Ljava/io/IOException;)V", 3, 1);
        let program = Program::new(std::slice::from_ref(&class), &[]).unwrap();
        let instructions = decode(&bytes).unwrap();
        let graph = ControlFlowGraph::build(&instructions, &[]).unwrap();
        let summary = analyze(&program, &class, &class.methods[0], &instructions, &graph).unwrap();
        assert_eq!(
            summary.throws[&4].source_type, "java/io/IOException",
            "{opcode:02x}"
        );
        assert!(
            ir::decompile_method(&class, &class.methods[0]).is_ok(),
            "{opcode:02x}"
        );
    }
}
