//! Small normal-result certificates, independent of a callee's effects.
//!
//! Every syntactic return must consume the same immediate integer literal.
//! A control-flow or exception entry at the return itself invalidates the
//! certificate. Calls, class initialization, synchronization and exceptions
//! remain observable even when this result can decide a caller's branch.
use crate::bytecode::{Operand, decode};
use crate::classfile::{ClassFile, Constant, JvmType, Method, MethodDescriptor};
use std::collections::BTreeSet;

pub(crate) fn constant_int(class: &ClassFile, method: &Method) -> Option<i32> {
    let result_type = MethodDescriptor::parse(&method.descriptor)
        .ok()?
        .return_type;
    if !matches!(
        result_type,
        JvmType::Boolean | JvmType::Byte | JvmType::Char | JvmType::Short | JvmType::Int
    ) {
        return None;
    }
    let code = method.code.as_ref()?;
    let instructions = decode(&code.bytes).ok()?;
    let mut entries: BTreeSet<_> = code
        .exception_table
        .iter()
        .map(|handler| handler.handler_pc)
        .collect();
    for instruction in &instructions {
        match &instruction.operand {
            Operand::Branch(target) => {
                entries.insert(*target);
            }
            Operand::Switch { default, cases } => {
                entries.insert(*default);
                entries.extend(cases.iter().map(|&(_, target)| target));
            }
            _ => {}
        }
    }
    let mut result = None;
    for (index, instruction) in instructions.iter().enumerate() {
        if !instruction.is_return() {
            continue;
        }
        if instruction.opcode != 0xac || entries.contains(&instruction.pc) {
            return None;
        }
        let previous = instructions.get(index.checked_sub(1)?)?;
        let value = match (previous.opcode, &previous.operand) {
            (0x02..=0x08, _) => i32::from(previous.opcode) - 3,
            (0x10 | 0x11, Operand::Int(value)) => *value,
            (0x12 | 0x13, Operand::Constant(index)) => {
                let Constant::Integer(value) = class.constant_pool.get(*index).ok()? else {
                    return None;
                };
                *value
            }
            _ => return None,
        };
        // Legacy IRETURN contracts do not specify modern narrowing behavior.
        // Certify only values already in the declared type's source range.
        let representable = match result_type {
            JvmType::Boolean => (0..=1).contains(&value),
            JvmType::Byte => i8::try_from(value).is_ok(),
            JvmType::Char => u16::try_from(value).is_ok(),
            JvmType::Short => i16::try_from(value).is_ok(),
            _ => true,
        };
        if !representable {
            return None;
        }
        if result.is_some_and(|previous| previous != value) {
            return None;
        }
        result = Some(value);
    }
    result
}
