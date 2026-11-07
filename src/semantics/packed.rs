//! Recognize physical slices without changing operations or conflating an
//! extracted field's meaning with the meaning of its encoded word.
use super::{
    PackedPart,
    flow::{Graph, NodeId, Op},
};
use crate::{classfile::JvmType, ir::BinaryOp, mappings::BitField};

pub(super) struct Extraction {
    pub source: NodeId,
    pub shift: u8,
    pub bits: u8,
    pub signed: bool,
    pub operands: Vec<(NodeId, PackedPart)>,
}
pub(super) struct Packing {
    pub value: NodeId,
    pub operands: Vec<(NodeId, PackedPart)>,
}
pub(super) struct Clearing {
    pub source: NodeId,
    pub shift: u8,
    pub bits: u8,
    pub constant: NodeId,
}

pub(super) fn width(graph: &Graph, node: NodeId) -> u8 {
    if graph.nodes[node].ty == JvmType::Long {
        64
    } else {
        32
    }
}
pub(super) fn low_mask(bits: u8) -> u64 {
    if bits == 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
}
fn low_bits(mask: i64, width: u8) -> Option<u8> {
    let mask = mask as u64 & low_mask(width);
    (mask != 0 && mask & mask.wrapping_add(1) == 0).then_some(mask.count_ones() as u8)
}
fn mask(graph: &Graph, node: NodeId) -> Option<(NodeId, NodeId)> {
    let Op::Binary {
        op: BinaryOp::And,
        left,
        right,
    } = graph.nodes[node].op
    else {
        return None;
    };
    if graph.integer(right).is_some() {
        Some((left, right))
    } else if graph.integer(left).is_some() {
        Some((right, left))
    } else {
        None
    }
}

/// An AND that clears exactly one contiguous physical slice. Restrict the
/// complement to the operation's width before identifying that slice: the
/// positive long 0xFFFFFF00L does not mean the same thing as the int -256.
pub(super) fn clearing(graph: &Graph, node: NodeId) -> Option<Clearing> {
    let width = width(graph, node);
    let (source, constant) = mask(graph, node)?;
    let cleared = !(graph.integer(constant)? as u64) & low_mask(width);
    if cleared == 0 {
        return None;
    }
    let shift = cleared.trailing_zeros() as u8;
    let bits = low_bits((cleared >> shift) as i64, width)?;
    Some(Clearing {
        source,
        shift,
        bits,
        constant,
    })
}
fn shifted(graph: &Graph, node: NodeId) -> Option<(NodeId, NodeId, bool)> {
    match graph.nodes[node].op {
        Op::Binary {
            op: BinaryOp::Shr,
            left,
            right,
        } => Some((left, right, true)),
        Op::Binary {
            op: BinaryOp::Ushr,
            left,
            right,
        } => Some((left, right, false)),
        _ => None,
    }
}

pub(super) fn extraction(graph: &Graph, node: NodeId) -> Option<Extraction> {
    let width = width(graph, node);
    if let Op::Cast { value, ref ty } = graph.nodes[node].op {
        let (bits, signed) = match ty {
            JvmType::Byte => (8, true),
            JvmType::Short => (16, true),
            JvmType::Char => (16, false),
            JvmType::Int if graph.nodes[value].ty == JvmType::Long => (32, true),
            _ => return None,
        };
        // JVM long-to-byte/short conversions can retain an intervening l2i.
        // Its low 32 bits are enough for the outer byte/short/char conversion.
        let mut value = value;
        if bits < 32
            && let Op::Cast {
                value: inner,
                ty: JvmType::Int,
            } = graph.nodes[value].op
        {
            value = inner;
        }
        if let Some(mut inner) = extraction(graph, value) {
            if inner.bits >= bits {
                // The cast interprets the final low bits, even when a mask
                // made the intermediate expression unsigned.
                inner.bits = bits;
                inner.signed = signed;
                return Some(inner);
            }
            if !inner.signed || signed {
                return Some(inner);
            }
        }
        let (source, shift, operands) = if let Some((source, amount, _)) = shifted(graph, value) {
            let shift = graph.integer(amount)? as u8 & (self::width(graph, value) - 1);
            (source, shift, vec![(amount, PackedPart::Shift)])
        } else {
            (value, 0, Vec::new())
        };
        return (u16::from(shift) + u16::from(bits) <= u16::from(self::width(graph, source)))
            .then_some(Extraction {
                source,
                shift,
                bits,
                signed,
                operands,
            });
    }
    if let Some((value, constant)) = mask(graph, node) {
        let bits = low_bits(graph.integer(constant)?, width)?;
        let (source, shift, operands) = if let Some((source, amount, _)) = shifted(graph, value) {
            let shift = graph.integer(amount)? as u8 & (width - 1);
            (
                source,
                shift,
                vec![
                    (
                        constant,
                        if shift == 0 {
                            PackedPart::Mask
                        } else {
                            PackedPart::ValueMask
                        },
                    ),
                    (amount, PackedPart::Shift),
                ],
            )
        } else {
            (value, 0, vec![(constant, PackedPart::Mask)])
        };
        return (u16::from(shift) + u16::from(bits) <= u16::from(width)).then_some(Extraction {
            source,
            shift,
            bits,
            signed: false,
            operands,
        });
    }
    if let Some((source, amount, signed)) = shifted(graph, node) {
        let shift = graph.integer(amount)? as u8 & (width - 1);
        if let Some((value, constant)) = mask(graph, source) {
            let stored = graph.integer(constant)? as u64 & low_mask(width);
            let bits = low_bits((stored >> shift) as i64, width)?;
            if stored & low_mask(shift) == 0 {
                let signed = signed && u16::from(shift) + u16::from(bits) == u16::from(width);
                return Some(Extraction {
                    source: value,
                    shift,
                    bits,
                    signed,
                    operands: vec![(constant, PackedPart::Mask), (amount, PackedPart::Shift)],
                });
            }
        }
        if let Op::Binary {
            op: BinaryOp::Shl,
            left,
            right,
        } = graph.nodes[source].op
        {
            let left_shift = graph.integer(right)? as u8 & (width - 1);
            if left_shift <= shift {
                return Some(Extraction {
                    source: left,
                    shift: shift - left_shift,
                    bits: width - shift,
                    signed,
                    operands: Vec::new(),
                });
            }
        }
        return Some(Extraction {
            source,
            shift,
            bits: width - shift,
            signed,
            operands: vec![(amount, PackedPart::Shift)],
        });
    }
    None
}

pub(super) fn packing(graph: &Graph, node: NodeId, field: &BitField) -> Option<Packing> {
    if u16::from(field.shift) + u16::from(field.bits) > u16::from(width(graph, node)) {
        return None;
    }
    let mut value = node;
    let mut operands = Vec::new();
    if field.shift != 0 {
        let Op::Binary {
            op: BinaryOp::Shl,
            left,
            right,
        } = graph.nodes[node].op
        else {
            return None;
        };
        if (graph.integer(right)? as u8 & (width(graph, node) - 1)) != field.shift {
            return None;
        }
        value = left;
        operands.push((right, PackedPart::Shift));
    }
    if let Some((inner, constant)) = mask(graph, value) {
        if low_bits(graph.integer(constant)?, width(graph, value)) != Some(field.bits) {
            return None;
        }
        value = inner;
        operands.push((
            constant,
            if field.shift == 0 {
                PackedPart::Mask
            } else {
                PackedPart::ValueMask
            },
        ));
    } else if field.shift == 0 {
        return None;
    }
    Some(Packing { value, operands })
}

#[cfg(test)]
#[path = "../../tests/unit/semantic_packed.rs"]
mod tests;
