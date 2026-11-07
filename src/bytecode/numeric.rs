//! Numeric opcode families describe physical operands and results. Abstract
//! domains decide whether to build an expression or retain only its category.
use super::stack;
use crate::{Result, classfile::JvmType};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BinaryOperation {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Shl,
    Shr,
    Ushr,
    And,
    Or,
    Xor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Comparison {
    Long,
    FloatLess,
    FloatGreater,
    DoubleLess,
    DoubleGreater,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NumericOperation {
    Binary {
        operation: BinaryOperation,
        ty: JvmType,
    },
    Negate(JvmType),
    Convert(JvmType),
    Compare(Comparison),
}

pub(crate) enum Operands<V> {
    Unary(V),
    Binary { left: V, right: V },
}

impl NumericOperation {
    pub fn decode(opcode: u8) -> Option<Self> {
        Some(match opcode {
            0x60..=0x73 => Self::Binary {
                operation: [
                    BinaryOperation::Add,
                    BinaryOperation::Sub,
                    BinaryOperation::Mul,
                    BinaryOperation::Div,
                    BinaryOperation::Rem,
                ][usize::from((opcode - 0x60) / 4)],
                ty: numeric_type((opcode - 0x60) % 4),
            },
            0x74..=0x77 => Self::Negate(numeric_type(opcode - 0x74)),
            0x78..=0x83 => Self::Binary {
                operation: [
                    BinaryOperation::Shl,
                    BinaryOperation::Shr,
                    BinaryOperation::Ushr,
                    BinaryOperation::And,
                    BinaryOperation::Or,
                    BinaryOperation::Xor,
                ][usize::from((opcode - 0x78) / 2)],
                ty: if opcode & 1 == 0 {
                    JvmType::Int
                } else {
                    JvmType::Long
                },
            },
            0x85..=0x93 => Self::Convert(
                [
                    JvmType::Long,
                    JvmType::Float,
                    JvmType::Double,
                    JvmType::Int,
                    JvmType::Float,
                    JvmType::Double,
                    JvmType::Int,
                    JvmType::Long,
                    JvmType::Double,
                    JvmType::Int,
                    JvmType::Long,
                    JvmType::Float,
                    JvmType::Byte,
                    JvmType::Char,
                    JvmType::Short,
                ][usize::from(opcode - 0x85)]
                .clone(),
            ),
            0x94..=0x98 => Self::Compare(
                [
                    Comparison::Long,
                    Comparison::FloatLess,
                    Comparison::FloatGreater,
                    Comparison::DoubleLess,
                    Comparison::DoubleGreater,
                ][usize::from(opcode - 0x94)],
            ),
            _ => return None,
        })
    }

    pub fn result_type(&self) -> JvmType {
        match self {
            Self::Binary { ty, .. } | Self::Negate(ty) | Self::Convert(ty) => ty.clone(),
            Self::Compare(_) => JvmType::Int,
        }
    }

    pub fn operands<V>(&self, stack: &mut Vec<V>) -> Result<Operands<V>> {
        let right = stack::pop(stack)?;
        Ok(match self {
            Self::Negate(_) | Self::Convert(_) => Operands::Unary(right),
            Self::Binary { .. } | Self::Compare(_) => Operands::Binary {
                left: stack::pop(stack)?,
                right,
            },
        })
    }
}

fn numeric_type(kind: u8) -> JvmType {
    match kind {
        0 => JvmType::Int,
        1 => JvmType::Long,
        2 => JvmType::Float,
        _ => JvmType::Double,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_families_have_correct_result_categories_and_operand_order() {
        // The expected categories are listed independently of the decoding
        // tables, including every widening/narrowing conversion and compare.
        for (first, categories, unary) in [
            (0x60, "12121212121212121212", false),
            (0x74, "1212", true),
            (0x78, "121212121212", false),
            (0x85, "212112122121111", true),
            (0x94, "11111", false),
        ] {
            for (offset, category) in categories.bytes().enumerate() {
                let opcode = first + offset as u8;
                let operation = NumericOperation::decode(opcode).unwrap();
                assert_eq!(
                    operation.result_type().slots(),
                    u16::from(category - b'0'),
                    "{opcode:02x}"
                );
                let mut stack = vec![0, 1, 2];
                match operation.operands(&mut stack).unwrap() {
                    Operands::Unary(value) => {
                        assert!(unary, "{opcode:02x}");
                        assert_eq!(value, 2);
                        assert_eq!(stack, [0, 1]);
                    }
                    Operands::Binary { left, right } => {
                        assert!(!unary, "{opcode:02x}");
                        assert_eq!((left, right), (1, 2));
                        assert_eq!(stack, [0]);
                    }
                }
            }
        }
        assert!(NumericOperation::decode(0x84).is_none()); // IINC is a local operation.
    }
}
