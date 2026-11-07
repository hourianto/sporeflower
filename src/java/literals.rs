use crate::{
    classfile::JvmType,
    ir::{BinaryOp, Literal, MethodIr, ValueId, ValueKind},
};

/// Print readable Unicode while preserving every UTF-16 code unit, including
/// deliberately unpaired surrogates in obfuscated constants.
pub fn string_literal(units: &[u16]) -> String {
    let mut result = String::with_capacity(units.len() + 2);
    result.push('"');
    for value in char::decode_utf16(units.iter().copied()) {
        match value {
            Ok(value) if value.len_utf16() == 2 && printable(value) => result.push(value),
            Ok(value) => {
                for unit in value.encode_utf16(&mut [0; 2]) {
                    escape_unit(*unit, false, &mut result);
                }
            }
            Err(error) => escape_unit(error.unpaired_surrogate(), false, &mut result),
        }
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

/// Keep invisible separators and non-text code points visible as escapes.
fn printable(value: char) -> bool {
    use unicode_general_category::{GeneralCategory, get_general_category};
    value == ' '
        || !matches!(
            get_general_category(value),
            GeneralCategory::Control
                | GeneralCategory::Format
                | GeneralCategory::SpaceSeparator
                | GeneralCategory::LineSeparator
                | GeneralCategory::ParagraphSeparator
                | GeneralCategory::PrivateUse
                | GeneralCategory::Unassigned
        )
}

fn escape_unit(value: u16, character: bool, out: &mut String) {
    use std::fmt::Write;
    let scalar = char::from_u32(u32::from(value));
    match value {
        8 => out.push_str("\\b"),
        9 => out.push_str("\\t"),
        10 => out.push_str("\\n"),
        12 => out.push_str("\\f"),
        13 => out.push_str("\\r"),
        34 if !character => out.push_str("\\\""),
        39 if character => out.push_str("\\'"),
        92 => out.push_str("\\\\"),
        _ if scalar.is_some_and(printable) => out.push(scalar.unwrap()),
        0..=255 => {
            // Unicode escapes are processed before Java tokenization. Octal
            // escapes cannot turn a control into a source line or delimiter.
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

/// These CLDC constants are presentation alternatives, so their owners use the
/// same optional name/import plan as authored semantic constants.
pub(crate) const STANDARD_OWNERS: &[&str] = &[
    "java/lang/Byte",
    "java/lang/Short",
    "java/lang/Integer",
    "java/lang/Long",
    "java/lang/Float",
    "java/lang/Double",
    "java/lang/Math",
];

/// Match the Java expression type as well as the exact value. Narrow limits
/// need an explicit byte/short context; 127 and 32767 are ordinary int values.
/// Character contexts retain character literals, including the NUL character.
/// Floating comparisons use bits so nearby values and signed zero never match.
pub(super) fn standard_constant(
    types: &super::types::TypeNames<'_>,
    literal: &Literal,
    ty: &JvmType,
) -> Option<super::syntax::Expr> {
    let (owner, name) = match (literal, ty) {
        (Literal::Int(-128), JvmType::Byte) => ("java/lang/Byte", "MIN_VALUE"),
        (Literal::Int(127), JvmType::Byte) => ("java/lang/Byte", "MAX_VALUE"),
        (Literal::Int(-32768), JvmType::Short) => ("java/lang/Short", "MIN_VALUE"),
        (Literal::Int(32767), JvmType::Short) => ("java/lang/Short", "MAX_VALUE"),
        (Literal::Int(i32::MIN), JvmType::Int) => ("java/lang/Integer", "MIN_VALUE"),
        (Literal::Int(i32::MAX), JvmType::Int) => ("java/lang/Integer", "MAX_VALUE"),
        (Literal::Long(i64::MIN), JvmType::Long) => ("java/lang/Long", "MIN_VALUE"),
        (Literal::Long(i64::MAX), JvmType::Long) => ("java/lang/Long", "MAX_VALUE"),
        (Literal::Float(value), JvmType::Float) => (
            "java/lang/Float",
            match value.to_bits() {
                0x0000_0001 => "MIN_VALUE",
                0x7f7f_ffff => "MAX_VALUE",
                0x7f80_0000 => "POSITIVE_INFINITY",
                0xff80_0000 => "NEGATIVE_INFINITY",
                0x7fc0_0000 => "NaN",
                _ => return None,
            },
        ),
        (Literal::Double(value), JvmType::Double) => match value.to_bits() {
            0x0000_0000_0000_0001 => ("java/lang/Double", "MIN_VALUE"),
            0x7fef_ffff_ffff_ffff => ("java/lang/Double", "MAX_VALUE"),
            0x7ff0_0000_0000_0000 => ("java/lang/Double", "POSITIVE_INFINITY"),
            0xfff0_0000_0000_0000 => ("java/lang/Double", "NEGATIVE_INFINITY"),
            0x7ff8_0000_0000_0000 => ("java/lang/Double", "NaN"),
            0x4009_21fb_5444_2d18 => ("java/lang/Math", "PI"),
            0x4005_bf0a_8b14_5769 => ("java/lang/Math", "E"),
            _ => return None,
        },
        _ => return None,
    };
    Some(super::syntax::Expr::atom(types.named_qualifier(owner)?).member(name))
}
