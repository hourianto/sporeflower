use crate::{
    classfile::JvmType,
    ir::{BinaryOp, Literal, MethodIr, ValueId, ValueKind},
};

/// Preserve UTF-16 code units, including deliberately unpaired surrogates in
/// obfuscated constants. Octal escapes avoid Java's pre-lexing Unicode escapes
/// turning line terminators into physical source lines.
pub fn string_literal(units: &[u16]) -> String {
    let mut result = String::with_capacity(units.len() + 2);
    result.push('"');
    for &unit in units {
        escape_unit(unit, false, &mut result);
    }
    result.push('"');
    result
}

pub fn char_literal(value: u16) -> String {
    let mut result = String::from("'");
    escape_unit(value, true, &mut result);
    result.push('\'');
    result
}

/// A character spelling is available only for an inline, representable integer.
/// The caller supplies character context; values outside the UTF-16 range keep
/// their numeric spelling instead of being narrowed.
pub(super) fn character_operand(ir: &MethodIr, value: ValueId) -> Option<String> {
    if ir.values[value].local.is_some() {
        return None;
    }
    let ValueKind::Literal(Literal::Int(value)) = ir.values[value].kind else {
        return None;
    };
    u16::try_from(value).ok().map(char_literal)
}

/// Integer add/subtract of a negative literal has the same width and evaluation
/// order with a positive RHS and the opposite operator. Checked negation keeps
/// MIN_VALUE valid without inventing an out-of-range positive Java literal.
/// Floating-point operations deliberately retain their original spelling.
pub(super) fn additive_operand(
    ir: &MethodIr,
    op: BinaryOp,
    value: ValueId,
    ty: &JvmType,
) -> Option<(BinaryOp, String)> {
    if !matches!(ty, JvmType::Int | JvmType::Long)
        || ir.values[value].local.is_some()
        || ir.values[value].ty == JvmType::Boolean
    {
        return None;
    }
    let opposite = match op {
        BinaryOp::Add => BinaryOp::Sub,
        BinaryOp::Sub => BinaryOp::Add,
        _ => return None,
    };
    let positive = match ir.values[value].kind {
        ValueKind::Literal(Literal::Int(value)) if value < 0 => value.checked_neg()?.to_string(),
        ValueKind::Literal(Literal::Long(value)) if value < 0 => {
            format!("{}L", value.checked_neg()?)
        }
        _ => return None,
    };
    Some((opposite, positive))
}

/// These int parameters denote a character, and String has no competing char
/// overload. Counts, offsets, general integer arguments, and supplementary code
/// points deliberately keep their numeric spelling.
pub fn character_argument(
    method: &crate::ir::Member,
    ir: &crate::ir::MethodIr,
    args: &[crate::ir::ValueId],
) -> Option<String> {
    if method.owner != "java/lang/String"
        || !matches!(method.name.as_str(), "indexOf" | "lastIndexOf")
        || !matches!(method.descriptor.as_str(), "(I)I" | "(II)I")
    {
        return None;
    }
    character_operand(ir, *args.first()?)
}

fn escape_unit(value: u16, character: bool, out: &mut String) {
    use std::fmt::Write;
    match value {
        8 => out.push_str("\\b"),
        9 => out.push_str("\\t"),
        10 => out.push_str("\\n"),
        12 => out.push_str("\\f"),
        13 => out.push_str("\\r"),
        34 if !character => out.push_str("\\\""),
        39 if character => out.push_str("\\'"),
        92 => out.push_str("\\\\"),
        32..=126 => out.push(char::from_u32(value as u32).unwrap()),
        0..=255 => {
            write!(out, "\\{value:03o}").unwrap();
        }
        _ => {
            write!(out, "\\u{value:04x}").unwrap();
        }
    }
}

pub fn float_literal(value: f32) -> String {
    if value.is_nan() {
        return "(0.0F / 0.0F)".into();
    }
    if value == f32::INFINITY {
        return "(1.0F / 0.0F)".into();
    }
    if value == f32::NEG_INFINITY {
        return "(-1.0F / 0.0F)".into();
    }
    let mut text = value.to_string();
    if !text.contains(['.', 'e', 'E']) {
        text.push_str(".0");
    }
    text.push('F');
    text
}

pub fn double_literal(value: f64) -> String {
    if value.is_nan() {
        return "(0.0D / 0.0D)".into();
    }
    if value == f64::INFINITY {
        return "(1.0D / 0.0D)".into();
    }
    if value == f64::NEG_INFINITY {
        return "(-1.0D / 0.0D)".into();
    }
    let mut text = value.to_string();
    if !text.contains(['.', 'e', 'E']) {
        text.push_str(".0");
    }
    text.push('D');
    text
}

pub fn primitive_literal(value: &Literal) -> Option<String> {
    match value {
        Literal::Null => Some("null".into()),
        Literal::Int(value) => Some(value.to_string()),
        Literal::Long(value) => Some(format!("{value}L")),
        Literal::Float(value) => Some(float_literal(*value)),
        Literal::Double(value) => Some(double_literal(*value)),
        Literal::String(value) => Some(string_literal(value)),
        Literal::Class(_) => None,
    }
}
