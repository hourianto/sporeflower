use crate::common::checked_stdout as checked;
const B0: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(0);
const B1: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(1);
const B2: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(2);
const B3: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(3);
const B4: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(4);
const B5: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(5);
const B6: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(6);
const B7: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(7);
use sporeflower::{
    classfile::ClassFile,
    ir,
    structure::{self, FlowBlock, FlowEnd, FlowGraph},
};
use std::{fs, process::Command};

fn utf(bytes: &mut Vec<u8>, value: &str) {
    bytes.push(1);
    bytes.extend((value.len() as u16).to_be_bytes());
    bytes.extend(value.as_bytes());
}

/// Reviewed version-49 bytecode: the initial range check enters either fold or
/// its following condition. Negative values loop through add then the same fold.
fn prefix_class() -> Vec<u8> {
    let code = [
        0x1a, 0x1b, 0xa1, 0, 9, // if (value < divisor) goto condition
        0x1a, 0x1b, 0xb8, 0, 12, 0x3b, // value = PrefixSupport.fold(value, divisor)
        0x1a, 0x9c, 0, 14, // condition: if (value >= 0) goto return
        0x1a, 0x1b, 0x11, 3, 0xe8, 0x68, 0x60, 0x3b, // value += divisor * 1000
        0xa7, 0xff, 0xee, // goto fold
        0x1a, 0xac,
    ];
    encode_class(&code, &[])
}

fn protected_prefix_class() -> Vec<u8> {
    let code = [
        0x1a, 0x1b, 0xa1, 0, 6, // first entry can bypass decrement
        0x84, 0, 0xff, // decrement: value-- (unprotected, nonthrowing)
        0x1a, 0x9e, 0, 12, // condition: if (value <= 0) goto return
        0x1a, 0x1b, 0xb8, 0, 12, 0x57, // protected fold effect
        0xa7, 0xff, 0xf3, // backedge always performs decrement
        0x1a, 0xac, 0x57, 0x10, 99, 0xac, // catch Throwable: return 99
    ];
    encode_class(&code, &[(12, 18, 23, 0)])
}

fn nested_protected_prefix_class() -> Vec<u8> {
    let code = [
        0x1a, 0x1b, 0xa1, 0, 6, // outer first entry may bypass its update
        0x84, 0, 0xff, // outer update: value--
        0x1a, 0x9e, 0, 30, // if value <= 0, return
        0x1b, 0x3d, // inner = divisor
        0x1a, 0x04, 0x7e, 0x99, 0, 6, // even value bypasses inner update
        0x84, 2, 0xff, // inner update: inner--
        0x1c, 0x9e, 0, 12, // if inner <= 0, outer backedge
        0x1a, 0x1c, 0xb8, 0, 12, 0x57, // protected effect(value, inner)
        0xa7, 0xff, 0xf3, // inner backedge
        0xa7, 0xff, 0xe1, // outer backedge
        0x1a, 0xac, 0x57, 0x10, 99, 0xac, // catch Throwable: return 99
    ];
    encode_class(&code, &[(27, 33, 41, 0)])
}

fn encode_class(code: &[u8], handlers: &[(u16, u16, u16, u16)]) -> Vec<u8> {
    let mut bytes = vec![0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 49, 0, 13];
    utf(&mut bytes, "PrefixLoop");
    bytes.extend([7, 0, 1]);
    utf(&mut bytes, "java/lang/Object");
    bytes.extend([7, 0, 3]);
    utf(&mut bytes, "run");
    utf(&mut bytes, "(II)I");
    utf(&mut bytes, "Code");
    utf(&mut bytes, "PrefixSupport");
    bytes.extend([7, 0, 8]);
    utf(&mut bytes, "fold");
    bytes.extend([12, 0, 10, 0, 6, 10, 0, 9, 0, 11]);
    bytes.extend([0, 0x31, 0, 2, 0, 4, 0, 0, 0, 0, 0, 1]);
    bytes.extend([0, 9, 0, 5, 0, 6, 0, 1, 0, 7]);
    bytes.extend(((code.len() + 12 + handlers.len() * 8) as u32).to_be_bytes());
    bytes.extend([0, 3, 0, 3]);
    bytes.extend((code.len() as u32).to_be_bytes());
    bytes.extend(code);
    bytes.extend((handlers.len() as u16).to_be_bytes());
    for &(start, end, target, catch_type) in handlers {
        for value in [start, end, target, catch_type] {
            bytes.extend(value.to_be_bytes());
        }
    }
    bytes.extend([0, 0, 0, 0]);
    bytes
}

#[test]
fn entry_prefix_effects_and_throws_match_on_initial_and_backedge_paths() {
    check_runtime_class(prefix_class());
}

#[test]
fn a_protected_loop_preserves_first_entry_bypass_and_caught_effects() {
    check_runtime_class(protected_prefix_class());
}

#[test]
fn nested_protected_loops_preserve_each_bypassed_update_and_exception_exit() {
    check_runtime_class(nested_protected_prefix_class());
}

#[test]
fn nested_planning_keeps_ineligible_original_prefixes_unmodified() {
    let class = ClassFile::parse(&nested_protected_prefix_class()).unwrap();
    let ir = ir::decompile_method(&class, &class.methods[0]).unwrap();
    let graph = FlowGraph::from(&ir);
    let prefixes: Vec<_> = graph
        .blocks
        .iter_enumerated()
        .filter_map(|(id, block)| block.is_loop_update.then_some(id))
        .collect();
    assert_eq!(prefixes.len(), 2);
    for prefix in prefixes {
        for cost in [None, Some(49)] {
            let mut unsupported = graph.clone();
            unsupported.blocks[prefix].duplication_cost = cost;
            assert_eq!(
                structure::structure_graph(&unsupported)
                    .unwrap()
                    .statistics
                    .dispatch_regions,
                1
            );
        }
    }
}

fn check_runtime_class(bytes: Vec<u8>) {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    let class = ClassFile::parse(&bytes).unwrap();
    let ir = ir::decompile_method(&class, &class.methods[0]).unwrap();
    let prepared = structure::PreparedIr::new(ir);
    let tree = structure::structure(&prepared).unwrap();
    assert_eq!(tree.statistics.dispatch_regions, 0, "{tree:#?}");
    fs::write(original.join("PrefixLoop.class"), bytes).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-g:none", "-cp"])
            .arg(&original)
            .arg("-d")
            .arg(&original)
            .arg("tests/fixtures/java/PrefixSupport.java"),
    );
    let expected = checked(
        Command::new(crate::common::java())
            .args(["-Xverify:all", "-cp"])
            .arg(&original)
            .arg("PrefixSupport"),
    );
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .args(["--no-api", "--threads", "1"]),
    );
    let rendered = fs::read_to_string(source.join("PrefixLoop.java")).unwrap();
    assert!(!rendered.contains("$state"), "{rendered}");
    // The original block's immutable values occur in two source scopes. Java
    // declaration placement must retain one binding rather than redeclare it.
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .arg(source.join("PrefixLoop.java"))
            .arg(source.join("PrefixSupport.java")),
    );
    let actual = checked(
        Command::new(crate::common::java())
            .args(["-Xverify:all", "-cp"])
            .arg(&rebuilt)
            .arg("PrefixSupport"),
    );
    assert_eq!(actual, expected);
}

fn entry_graph(cost: Option<u16>) -> FlowGraph {
    let ends = vec![
        FlowEnd::Branch {
            then_target: B1,
            else_target: B2,
        },
        FlowEnd::Goto(B2),
        FlowEnd::Branch {
            then_target: B3,
            else_target: B4,
        },
        FlowEnd::Goto(B1),
        FlowEnd::Return,
    ];
    FlowGraph {
        entry: B0,
        blocks: ends
            .into_iter()
            .enumerate()
            .map(|(id, end)| FlowBlock {
                pc: id as u32,
                statements_empty: false,
                duplication_cost: (id == 1).then_some(cost).flatten(),
                is_loop_update: false,
                end,
                handlers: vec![],
                may_throw: false,
            })
            .collect(),
    }
}

#[test]
fn unsupported_or_expensive_prefixes_keep_the_original_dispatcher() {
    for cost in [None, Some(49)] {
        assert_eq!(
            structure::structure_graph(&entry_graph(cost))
                .unwrap()
                .statistics
                .dispatch_regions,
            1
        );
    }
    assert_eq!(
        structure::structure_graph(&entry_graph(Some(2)))
            .unwrap()
            .statistics
            .dispatch_regions,
        0
    );
    let mut handler_entry = entry_graph(Some(2));
    handler_entry.blocks[B0]
        .handlers
        .push(sporeflower::cfg::HandlerEdge {
            target: B1,
            catch_type: None,
            table_index: 0,
        });
    assert_eq!(
        structure::structure_graph(&handler_entry)
            .unwrap()
            .statistics
            .dispatch_regions,
        1
    );
}

#[test]
fn recovery_history_explains_the_original_cycle_and_selected_fallback() {
    use structure::{RecoveryAttempt, RecoveryRejection, RecoveryStrategy, RejectionReason};

    for cost in [None, Some(2)] {
        let tree = structure::structure_graph(&entry_graph(cost)).unwrap();
        assert_eq!(
            tree.recovery_attempts[..2],
            [
                RecoveryAttempt {
                    entry: B0,
                    strategy: RecoveryStrategy::Ordinary,
                    rejection: Some(RecoveryRejection {
                        block: B1,
                        reason: RejectionReason::RegionBoundary,
                    }),
                },
                RecoveryAttempt {
                    entry: B0,
                    strategy: RecoveryStrategy::Lexical,
                    rejection: Some(RecoveryRejection {
                        block: B1,
                        reason: RejectionReason::CyclicOrder,
                    }),
                },
            ]
        );
        let selected = tree.recovery_attempts.last().unwrap();
        assert_eq!(selected.rejection, None);
        assert_eq!(
            selected.strategy,
            if cost.is_some() {
                RecoveryStrategy::EntryDuplication
            } else {
                RecoveryStrategy::Dispatch
            }
        );
        assert!(tree.recovery_attempts.iter().all(|attempt| {
            attempt.entry < B5 && attempt.rejection.is_none_or(|reason| reason.block < B5)
        }));
        if cost.is_none() {
            assert_eq!(
                tree.recovery_attempts[2].rejection,
                Some(RecoveryRejection {
                    block: B1,
                    reason: RejectionReason::EntryPrefix,
                })
            );
            assert!(
                tree.recovery_attempts[2]
                    .to_string()
                    .contains("not applicable")
            );
        }
    }
}

#[test]
fn a_nested_natural_loop_backedge_is_not_an_extra_entry() {
    let ends = [
        FlowEnd::Branch {
            then_target: B1,
            else_target: B2,
        },
        FlowEnd::Goto(B2),
        FlowEnd::Branch {
            then_target: B3,
            else_target: B7,
        },
        FlowEnd::Goto(B4),
        FlowEnd::Branch {
            then_target: B5,
            else_target: B6,
        },
        FlowEnd::Goto(B4),
        FlowEnd::Goto(B1),
        FlowEnd::Return,
    ];
    let graph = FlowGraph {
        entry: B0,
        blocks: ends
            .into_iter()
            .enumerate()
            .map(|(id, end)| FlowBlock {
                pc: id as u32,
                statements_empty: false,
                duplication_cost: (id == 1).then_some(2),
                is_loop_update: false,
                end,
                handlers: vec![],
                may_throw: false,
            })
            .collect(),
    };
    assert_eq!(
        structure::structure_graph(&graph)
            .unwrap()
            .statistics
            .dispatch_regions,
        0
    );
}

#[test]
fn expanded_expression_cost_excludes_large_inline_payloads() {
    let class = ClassFile::parse(&prefix_class()).unwrap();
    let mut ir = ir::decompile_method(&class, &class.methods[0]).unwrap();
    let candidate = FlowGraph::from(&ir)
        .blocks
        .iter_enumerated()
        .find_map(|(id, block)| block.duplication_cost.is_some().then_some(id))
        .unwrap();
    let value = match ir.blocks[candidate].statements[0] {
        ir::Statement::Assign { value, .. } => value.value(),
        ref other => panic!("unexpected prefix {other:?}"),
    };
    ir.values[value].kind = ir::ValueKind::Literal(ir::Literal::String(vec![b'x' as u16; 4096]));
    assert!(
        FlowGraph::from(&ir).blocks[candidate]
            .duplication_cost
            .is_none()
    );
}
