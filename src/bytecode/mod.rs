//! Complete legacy JVM instruction decoding; all branch operands are absolute byte offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instruction {
    pub pc: u32,
    pub opcode: u8,
    pub operand: Operand,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operand {
    None,
    Int(i32),
    Constant(u16),
    Local(u16),
    Increment {
        local: u16,
        amount: i16,
    },
    Branch(u32),
    Switch {
        default: u32,
        cases: Vec<(i32, u32)>,
    },
    InterfaceCall {
        index: u16,
        count: u8,
    },
    MultiArray {
        index: u16,
        dimensions: u8,
    },
}

mod decode;
pub use decode::decode;
pub(crate) mod numeric;
pub(crate) mod stack;

impl Instruction {
    pub(crate) fn constant_index(&self) -> crate::Result<u16> {
        match self.operand {
            Operand::Constant(index)
            | Operand::InterfaceCall { index, .. }
            | Operand::MultiArray { index, .. } => Ok(index),
            _ => anyhow::bail!("missing constant-pool operand at {}", self.pc),
        }
    }

    pub(crate) fn local_index(&self) -> crate::Result<u16> {
        match self.operand {
            Operand::Local(index) => Ok(index),
            _ => anyhow::bail!("missing local operand at {}", self.pc),
        }
    }

    pub(crate) fn newarray_type(&self) -> crate::Result<crate::classfile::JvmType> {
        use crate::classfile::JvmType;
        let Operand::Int(kind) = self.operand else {
            anyhow::bail!("missing newarray element type at {}", self.pc)
        };
        Ok(match kind {
            4 => JvmType::Boolean,
            5 => JvmType::Char,
            6 => JvmType::Float,
            7 => JvmType::Double,
            8 => JvmType::Byte,
            9 => JvmType::Short,
            10 => JvmType::Int,
            11 => JvmType::Long,
            _ => anyhow::bail!("invalid newarray element type {kind} at {}", self.pc),
        })
    }

    /// Whether this opcode can transfer to an exception handler. Analyses may
    /// refine constant resolution using the referenced constant-pool entry.
    pub fn can_throw(&self) -> bool {
        matches!(self.opcode, 0x12..=0x14 | 0x2e..=0x35 | 0x4f..=0x56 | 0x6c | 0x6d | 0x70 | 0x71 | 0xb2..=0xc3 | 0xc5)
    }
    pub fn is_conditional(&self) -> bool {
        matches!(self.opcode, 0x99..=0xa6 | 0xc6 | 0xc7)
    }
    pub fn is_return(&self) -> bool {
        matches!(self.opcode, 0xac..=0xb1)
    }
    pub fn can_fall_through(&self) -> bool {
        !matches!(self.opcode, 0xa7..=0xab | 0xac..=0xb1 | 0xbf)
    }
    pub fn branch_target(&self) -> Option<u32> {
        if let Operand::Branch(pc) = self.operand {
            Some(pc)
        } else {
            None
        }
    }
}
