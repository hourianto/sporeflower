use super::*;
use crate::ir::Literal;
use crate::semantics::ranges::test_graph;

fn int(number: i32) -> (JvmType, Op) {
    (JvmType::Int, Op::Literal(Literal::Int(number)))
}
fn field(shift: u8, bits: u8) -> BitField {
    BitField {
        domain: None,
        shift,
        bits,
        signed: false,
        selector_mask: 0,
        selector_value: 0,
        name: None,
    }
}

#[test]
fn top_mask_then_arithmetic_shift_preserves_signedness() {
    for (ty, mask, shift, bits) in [
        (JvmType::Int, -65536i64, 16, 16),
        (JvmType::Long, i64::MIN, 63, 1),
    ] {
        let constant = if ty == JvmType::Long {
            Literal::Long(mask)
        } else {
            Literal::Int(mask as i32)
        };
        let graph = test_graph(vec![
            (ty.clone(), Op::Parameter(0)),
            (ty.clone(), Op::Literal(constant)),
            (
                ty.clone(),
                Op::Binary {
                    op: BinaryOp::And,
                    left: 0,
                    right: 1,
                },
            ),
            int(shift),
            (
                ty.clone(),
                Op::Binary {
                    op: BinaryOp::Shr,
                    left: 2,
                    right: 3,
                },
            ),
            (
                ty,
                Op::Binary {
                    op: BinaryOp::Ushr,
                    left: 2,
                    right: 3,
                },
            ),
        ]);
        for (node, signed) in [(4, true), (5, false)] {
            let extraction = extraction(&graph, node).unwrap();
            assert_eq!(
                (
                    extraction.source,
                    extraction.shift,
                    extraction.bits,
                    extraction.signed
                ),
                (0, shift as u8, bits, signed)
            );
        }
    }
}

#[test]
fn narrowing_long_slices_retains_long_shift_width() {
    let graph = test_graph(vec![
        (JvmType::Long, Op::Parameter(0)),
        int(40),
        (
            JvmType::Long,
            Op::Binary {
                op: BinaryOp::Ushr,
                left: 0,
                right: 1,
            },
        ),
        (
            JvmType::Int,
            Op::Cast {
                value: 2,
                ty: JvmType::Int,
            },
        ),
        (
            JvmType::Byte,
            Op::Cast {
                value: 3,
                ty: JvmType::Byte,
            },
        ),
    ]);
    let extraction = extraction(&graph, 4).unwrap();
    assert_eq!(
        (
            extraction.source,
            extraction.shift,
            extraction.bits,
            extraction.signed
        ),
        (0, 40, 8, true)
    );
}

#[test]
fn narrow_cast_after_mask_controls_the_extracted_signedness() {
    let graph = test_graph(vec![
        (JvmType::Int, Op::Parameter(0)),
        int(16),
        (
            JvmType::Int,
            Op::Binary {
                op: BinaryOp::Ushr,
                left: 0,
                right: 1,
            },
        ),
        int(65535),
        (
            JvmType::Int,
            Op::Binary {
                op: BinaryOp::And,
                left: 2,
                right: 3,
            },
        ),
        (
            JvmType::Short,
            Op::Cast {
                value: 4,
                ty: JvmType::Short,
            },
        ),
    ]);
    let extraction = extraction(&graph, 5).unwrap();
    assert_eq!(
        (
            extraction.source,
            extraction.shift,
            extraction.bits,
            extraction.signed
        ),
        (0, 16, 16, true)
    );
}

#[test]
fn shift_pairs_match_signed_and_unsigned_physical_slices() {
    let graph = test_graph(vec![
        (JvmType::Int, Op::Parameter(0)),
        int(20),
        (
            JvmType::Int,
            Op::Binary {
                op: BinaryOp::Shl,
                left: 0,
                right: 1,
            },
        ),
        int(24),
        (
            JvmType::Int,
            Op::Binary {
                op: BinaryOp::Shr,
                left: 2,
                right: 3,
            },
        ),
        (
            JvmType::Int,
            Op::Binary {
                op: BinaryOp::Ushr,
                left: 2,
                right: 3,
            },
        ),
    ]);
    for (node, signed) in [(4, true), (5, false)] {
        let extraction = extraction(&graph, node).unwrap();
        assert_eq!(
            (
                extraction.source,
                extraction.shift,
                extraction.bits,
                extraction.signed
            ),
            (0, 4, 8, signed)
        );
        assert!(extraction.operands.is_empty());
    }
}

#[test]
fn unrelated_arithmetic_is_not_low_field_packing() {
    let graph = test_graph(vec![
        (JvmType::Int, Op::Parameter(0)),
        int(1),
        (
            JvmType::Int,
            Op::Binary {
                op: BinaryOp::Add,
                left: 0,
                right: 1,
            },
        ),
        int(15),
        (
            JvmType::Int,
            Op::Binary {
                op: BinaryOp::And,
                left: 0,
                right: 3,
            },
        ),
    ]);
    assert!(packing(&graph, 2, &field(0, 4)).is_none());
    assert_eq!(packing(&graph, 4, &field(0, 4)).unwrap().value, 0);
    assert!(packing(&graph, 2, &field(0, 40)).is_none());
}

#[test]
fn clear_masks_use_the_operation_width_and_one_contiguous_slice() {
    for (ty, mask, expected) in [
        (JvmType::Int, -256, Some((0, 8))),
        (JvmType::Long, -256, Some((0, 8))),
        (JvmType::Long, 4294967040, None),
        (JvmType::Int, i32::MAX as i64, Some((31, 1))),
        (JvmType::Long, i64::MAX, Some((63, 1))),
        (JvmType::Long, -4294967296, Some((0, 32))),
        (JvmType::Int, 0, Some((0, 32))),
        (JvmType::Long, 0, Some((0, 64))),
        (JvmType::Int, -1, None),
        (JvmType::Long, -71776119061217281, Some((48, 8))),
    ] {
        let literal = if ty == JvmType::Long {
            Literal::Long(mask)
        } else {
            Literal::Int(mask as i32)
        };
        let graph = test_graph(vec![
            (ty.clone(), Op::Parameter(0)),
            (ty.clone(), Op::Literal(literal)),
            (
                ty,
                Op::Binary {
                    op: BinaryOp::And,
                    left: 0,
                    right: 1,
                },
            ),
        ]);
        assert_eq!(
            clearing(&graph, 2).map(|result| {
                assert_eq!((result.source, result.constant), (0, 1));
                (result.shift, result.bits)
            }),
            expected,
            "clear mask {mask}"
        );
    }
}
