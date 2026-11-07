use sporeflower::bytecode::{self, Operand};
use sporeflower::cfg;
use sporeflower::classfile::{ClassFile, ExceptionHandler, JvmType, MethodDescriptor};
use sporeflower::index::BlockId;

fn u2(output: &mut Vec<u8>, value: u16) {
    output.extend(value.to_be_bytes());
}
fn u4(output: &mut Vec<u8>, value: u32) {
    output.extend(value.to_be_bytes());
}
fn utf(output: &mut Vec<u8>, value: &[u8]) {
    output.push(1);
    u2(output, value.len() as u16);
    output.extend(value);
}

fn fixture_class(string: &[u8], malformed_throws: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    u4(&mut bytes, 0xcafe_babe);
    u2(&mut bytes, 3);
    u2(&mut bytes, 45);
    u2(&mut bytes, 11);
    utf(&mut bytes, b"pkg/Fixture");
    bytes.push(7);
    u2(&mut bytes, 1);
    utf(&mut bytes, b"java/lang/Object");
    bytes.push(7);
    u2(&mut bytes, 3);
    utf(&mut bytes, b"value");
    utf(&mut bytes, b"()I");
    utf(&mut bytes, b"Code");
    utf(&mut bytes, b"Exceptions");
    utf(&mut bytes, string);
    bytes.push(8);
    u2(&mut bytes, 9);
    u2(&mut bytes, 0x21);
    u2(&mut bytes, 2);
    u2(&mut bytes, 4);
    u2(&mut bytes, 0);
    u2(&mut bytes, 0);
    u2(&mut bytes, 1);
    u2(&mut bytes, 9);
    u2(&mut bytes, 5);
    u2(&mut bytes, 6);
    u2(&mut bytes, if malformed_throws { 2 } else { 1 });
    u2(&mut bytes, 7);
    u4(&mut bytes, 15);
    u2(&mut bytes, 1);
    u2(&mut bytes, 0);
    u4(&mut bytes, 3);
    bytes.extend([0x10, 42, 0xac]);
    u2(&mut bytes, 0);
    u2(&mut bytes, 0);
    if malformed_throws {
        u2(&mut bytes, 8);
        u4(&mut bytes, 6);
        u2(&mut bytes, 2);
        u2(&mut bytes, 10);
        u2(&mut bytes, 65535);
    }
    u2(&mut bytes, 0);
    bytes
}

#[test]
fn parser_preserves_modified_utf8_code_units() {
    let bytes = fixture_class(
        &[
            b'a', 0xc0, 0x80, 0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80, 0xed, 0xa0, 0x80,
        ],
        false,
    );
    let class = ClassFile::parse(&bytes).unwrap();
    assert_eq!(
        class.constant_pool.string_utf16(10).unwrap(),
        [0x61, 0, 0xd83d, 0xde00, 0xd800]
    );
    assert_eq!(
        class.methods[0].code.as_ref().unwrap().bytes,
        [0x10, 42, 0xac]
    );
}

#[test]
fn broken_throws_metadata_does_not_discard_method() {
    let class = ClassFile::parse(&fixture_class(b"x", true)).unwrap();
    assert!(class.methods[0].exceptions.is_empty());
    assert_eq!(class.warnings.len(), 2);
    assert!(class.methods[0].code.is_some());
}

fn metadata_class(attribute_copies: &[bool]) -> Vec<u8> {
    fn attributes(output: &mut Vec<u8>, entries: &[(u16, &[u16])], copies: &[bool]) {
        u2(output, (entries.len() * copies.len()) as u16);
        for &malformed in copies {
            for &(name, words) in entries {
                u2(output, name);
                u4(output, (words.len() * 2 + usize::from(malformed)) as u32);
                for &word in words {
                    u2(output, word);
                }
                if malformed {
                    output.push(0);
                }
            }
        }
    }

    let mut bytes = Vec::new();
    u4(&mut bytes, 0xcafe_babe);
    u2(&mut bytes, 0);
    u2(&mut bytes, 49);
    u2(&mut bytes, 18);
    utf(&mut bytes, b"Metadata"); // 1
    bytes.push(7);
    u2(&mut bytes, 1); // 2: class
    utf(&mut bytes, b"java/lang/Object"); // 3
    bytes.push(7);
    u2(&mut bytes, 3); // 4: superclass / throws entry
    for text in [
        "value",
        "I",
        "method",
        "()V",
        "Code",
        "ConstantValue",
        "SourceFile",
        "Metadata.java",
        "InnerClasses",
        "LineNumberTable",
        "LocalVariableTable",
        "Exceptions",
    ] {
        utf(&mut bytes, text.as_bytes()); // 5..16
    }
    bytes.push(3);
    u4(&mut bytes, 42); // 17: field constant
    for value in [0x21, 2, 4, 0, 1, 0x19, 5, 6] {
        u2(&mut bytes, value);
    }
    attributes(&mut bytes, &[(10, &[17])], attribute_copies);
    for value in [1, 0x09, 7, 8] {
        u2(&mut bytes, value);
    }
    let mut code = Vec::new();
    u2(&mut code, 0);
    u2(&mut code, 1);
    u4(&mut code, 1);
    code.push(0xb1);
    u2(&mut code, 0);
    attributes(
        &mut code,
        &[(14, &[1, 0, 7]), (15, &[1, 0, 1, 5, 6, 0])],
        attribute_copies,
    );
    u2(&mut bytes, (1 + attribute_copies.len()) as u16);
    u2(&mut bytes, 9);
    u4(&mut bytes, code.len() as u32);
    bytes.extend(code);
    // Add the throws attributes after Code without another attribute count.
    let mut exceptions = Vec::new();
    attributes(&mut exceptions, &[(16, &[1, 4])], attribute_copies);
    bytes.extend(&exceptions[2..]);
    attributes(
        &mut bytes,
        &[(11, &[12]), (13, &[1, 2, 0, 5, 0])],
        attribute_copies,
    );
    bytes
}

#[test]
fn ignored_metadata_is_not_partially_installed() {
    let class = ClassFile::parse(&metadata_class(&[true])).unwrap();
    assert_eq!(class.warnings.len(), 6);
    assert!(
        class
            .warnings
            .iter()
            .all(|warning| warning.contains("ignored malformed"))
    );
    assert!(class.fields[0].constant_value.is_none());
    assert!(class.source_file.is_none());
    assert!(class.inner_classes.is_empty());
    assert!(class.methods[0].exceptions.is_empty());
    let code = class.methods[0].code.as_ref().unwrap();
    assert!(code.line_numbers.is_empty());
    assert!(code.local_variables.is_empty());
    assert_eq!(code.bytes, [0xb1]);
}

#[test]
fn malformed_metadata_does_not_erase_valid_attributes() {
    for copies in [&[false][..], &[false, true], &[true, false]] {
        let class = ClassFile::parse(&metadata_class(copies)).unwrap();
        assert_eq!(class.fields[0].constant_value, Some(17));
        assert_eq!(class.source_file.as_deref(), Some("Metadata.java"));
        assert_eq!(class.inner_classes.len(), 1);
        assert_eq!(class.methods[0].exceptions, ["java/lang/Object"]);
        let code = class.methods[0].code.as_ref().unwrap();
        assert_eq!(code.line_numbers.len(), 1);
        assert_eq!(code.local_variables.len(), 1);
    }
}

#[test]
fn every_truncation_returns_an_error_without_panicking() {
    let bytes = fixture_class(b"x", false);
    for length in 0..bytes.len() {
        assert!(
            ClassFile::parse(&bytes[..length]).is_err(),
            "accepted truncation {length}"
        );
    }
}

#[test]
fn descriptor_round_trip_preserves_narrow_types_and_arrays() {
    let descriptor = "(ZBCSIJFD[[Ljava/lang/String;)[I";
    let parsed = MethodDescriptor::parse(descriptor).unwrap();
    assert_eq!(parsed.descriptor(), descriptor);
    assert_eq!(parsed.parameter_slots(), 11);
    assert_eq!(parsed.return_type, JvmType::Array(Box::new(JvmType::Int)));
    for invalid in ["(V)V", "(I", "()Vx", "([V)V", "(L;)V"] {
        assert!(
            MethodDescriptor::parse(invalid).is_err(),
            "accepted {invalid}"
        );
    }
    assert!(JvmType::parse(&format!("{}I", "[".repeat(256))).is_err());
}

#[test]
fn compact_wide_and_signed_operands_are_normalized() {
    let code = bytecode::decode(&[
        0x2d, 0x4e, 0xc4, 0x15, 1, 2, 0xc4, 0x84, 1, 2, 0xff, 0xfe, 0x10, 0xff, 0xb1,
    ])
    .unwrap();
    assert_eq!(
        (code[0].opcode, &code[0].operand),
        (0x19, &Operand::Local(3))
    );
    assert_eq!(
        (code[1].opcode, &code[1].operand),
        (0x3a, &Operand::Local(3))
    );
    assert_eq!(code[2].operand, Operand::Local(258));
    assert_eq!(
        code[3].operand,
        Operand::Increment {
            local: 258,
            amount: -2
        }
    );
    assert_eq!(code[4].operand, Operand::Int(-1));
    assert_eq!(code[4].pc, 12);
}

#[test]
fn branches_into_operands_and_truncated_switches_are_rejected() {
    assert!(bytecode::decode(&[0xa7, 0, 1, 0xb1]).is_err());
    assert!(bytecode::decode(&[0xc4, 0x00]).is_err());
    assert!(bytecode::decode(&[0xab, 0, 0, 0, 0, 0, 0, 0, 0x7f, 0xff, 0xff, 0xff]).is_err());
    assert!(
        bytecode::decode(&[
            0xaa, 0, 0, 0, 0, 0, 0, 0, 0x80, 0, 0, 0, 0x7f, 0xff, 0xff, 0xff
        ])
        .is_err()
    );
}

#[test]
fn tableswitch_alignment_and_shared_targets_survive_decoding() {
    let mut bytes = vec![0x03, 0xaa, 0, 0];
    for value in [23i32, -1, 0, 23, 23] {
        bytes.extend(value.to_be_bytes());
    }
    bytes.push(0xb1);
    let code = bytecode::decode(&bytes).unwrap();
    assert_eq!(
        code[1].operand,
        Operand::Switch {
            default: 24,
            cases: vec![(-1, 24), (0, 24)]
        }
    );
    let graph = cfg::build(&code, &[]).unwrap();
    assert_eq!(
        graph.blocks[BlockId::from_index(0)].successors,
        [BlockId::from_index(1)]
    );
}

#[test]
fn cfg_retains_exception_priority_and_boundary_changes() {
    let code = bytecode::decode(&[0x00, 0x00, 0xb1, 0xbf, 0xbf]).unwrap();
    let exceptions = vec![
        ExceptionHandler {
            start_pc: 0,
            end_pc: 2,
            handler_pc: 3,
            catch_type: None,
        },
        ExceptionHandler {
            start_pc: 1,
            end_pc: 2,
            handler_pc: 4,
            catch_type: Some("java/lang/Exception".into()),
        },
    ];
    let graph = cfg::build(&code, &exceptions).unwrap();
    assert_eq!(graph.blocks[BlockId::from_index(0)].handlers.len(), 1);
    assert_eq!(
        graph.blocks[BlockId::from_index(1)]
            .handlers
            .iter()
            .map(|h| h.table_index)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert!(graph.blocks[BlockId::from_index(2)].handlers.is_empty());
    assert_eq!(
        graph.blocks[BlockId::from_index(1)].predecessors,
        [BlockId::from_index(0)]
    );
    assert!(graph.blocks[BlockId::from_index(3)].predecessors.is_empty());
}

#[test]
fn cfg_allows_dead_padding_but_rejects_reachable_fallthrough() {
    assert!(cfg::build(&bytecode::decode(&[0xb1, 0x00]).unwrap(), &[]).is_ok());
    assert!(cfg::build(&bytecode::decode(&[0x00]).unwrap(), &[]).is_err());
}

fn follow_normalized(code: &[sporeflower::bytecode::Instruction]) -> Vec<u32> {
    let mut pc = 0;
    let mut visited = Vec::new();
    for _ in 0..100 {
        let instruction = &code[pc as usize];
        visited.push(pc);
        if instruction.opcode == 0xb1 {
            return visited;
        }
        pc = match instruction.operand {
            Operand::Branch(target) if instruction.opcode == 0xa7 => target,
            _ => pc + 1,
        };
    }
    panic!("normalized subroutine did not terminate")
}

#[test]
fn shared_subroutine_returns_to_each_call_site() {
    let decoded =
        bytecode::decode(&[0xa8, 0, 7, 0xa8, 0, 4, 0xb1, 0x4b, 0x84, 1, 1, 0xa9, 0]).unwrap();
    let normalized = cfg::normalize_subroutines(&decoded, &[]).unwrap();
    assert!(
        !normalized
            .instructions
            .iter()
            .any(|i| matches!(i.opcode, 0xa8 | 0xa9 | 0x3a))
    );
    let path = follow_normalized(&normalized.instructions);
    assert_eq!(
        path.iter()
            .filter(|&&pc| normalized.instructions[pc as usize].opcode == 0x84)
            .count(),
        2
    );
    cfg::build(&normalized.instructions, &normalized.exception_table).unwrap();
}

#[test]
fn nested_subroutines_keep_both_return_addresses() {
    let decoded = bytecode::decode(&[
        0xa8, 0, 4, 0xb1, 0x4b, 0xa8, 0, 5, 0xa9, 0, 0x4c, 0x84, 2, 1, 0xa9, 1,
    ])
    .unwrap();
    let normalized = cfg::normalize_subroutines(&decoded, &[]).unwrap();
    let path = follow_normalized(&normalized.instructions);
    assert_eq!(
        path.iter()
            .filter(|&&pc| normalized.instructions[pc as usize].opcode == 0x84)
            .count(),
        1
    );
    cfg::build(&normalized.instructions, &normalized.exception_table).unwrap();
}

#[test]
fn actual_recursive_subroutines_are_still_rejected() {
    // Main calls byte4; that subroutine calls itself without leaving its body.
    let decoded = bytecode::decode(&[0xa8, 0, 4, 0xb1, 0x4b, 0xa8, 0xff, 0xff, 0xa9, 0]).unwrap();
    let error = cfg::normalize_subroutines(&decoded, &[]).unwrap_err();
    assert!(error.to_string().contains("recursive jsr"), "{error}");
}

#[test]
fn subroutine_handler_clones_keep_caller_context_and_coverage() {
    let mut code = sporeflower::classfile::Code {
        max_stack: 1,
        max_locals: 2,
        bytes: vec![
            0xa8, 0, 7, 0xa8, 0, 4, 0xb1, 0x4b, 0x01, 0xbf, 0x4c, 0xa9, 0,
        ],
        exception_table: vec![ExceptionHandler {
            start_pc: 8,
            end_pc: 10,
            handler_pc: 10,
            catch_type: Some("java/lang/Throwable".into()),
        }],
        line_numbers: Vec::new(),
        local_variables: vec![sporeflower::classfile::LocalVariable {
            start_pc: 11,
            length: 2,
            name: "caught".into(),
            descriptor: "Ljava/lang/Throwable;".into(),
            index: 1,
        }],
    };
    let prepared = cfg::PreparedCode::new(&code).unwrap();
    let graph = &prepared.graph;
    let throws: Vec<_> = prepared
        .instructions
        .iter()
        .enumerate()
        .filter(|(_, i)| i.opcode == 0xbf)
        .collect();
    assert_eq!(throws.len(), 2);
    let first = &graph.blocks[graph.instruction_blocks[throws[0].0]].handlers;
    let second = &graph.blocks[graph.instruction_blocks[throws[1].0]].handlers;
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert_ne!(first[0].target, second[0].target);
    for (index, instruction) in throws {
        assert_eq!(prepared.original_pc(index), 9);
        assert_eq!(
            prepared.debug_ranges().unwrap()[instruction.pc as usize],
            (9, 10)
        );
    }
    // RET is replaced with a jump, but its original span still ends at the
    // original bytecode length rather than at a normalized instruction index.
    assert!(prepared.debug_ranges().unwrap().contains(&(11, 13)));
    code.local_variables.clear();
    let prepared = cfg::PreparedCode::new(&code).unwrap();
    assert_eq!(prepared.debug_ranges(), Some([].as_slice()));
}
