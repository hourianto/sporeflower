//! Numeric contracts shared by bytecode update recovery and local source assignments.
use super::type_facts::{numeric, promoted, widening};
use super::{BinaryOp, JvmType, MethodIr, ValueId, ValueKind};

pub(crate) struct Computation<'a> {
    pub op: BinaryOp,
    pub read: ValueId,
    pub right: ValueId,
    pub ty: &'a JvmType,
}

pub(crate) fn computation<'a>(
    ir: &'a MethodIr,
    value: ValueId,
    target: &JvmType,
    definition: bool,
) -> Option<Computation<'a>> {
    if !numeric(target) && *target != JvmType::Boolean {
        return None;
    }
    let mut value = if definition {
        value
    } else {
        inline_alias(ir, value)?
    };
    if let ValueKind::Cast { value: inner, ty } = &ir.values[value].kind {
        if ty != target {
            return None;
        }
        value = inline_alias(ir, *inner)?;
        // JVM conversion from float/double/long to byte, short or char goes
        // through int. Java's compound-assignment narrowing performs both.
        if matches!(target, JvmType::Byte | JvmType::Short | JvmType::Char)
            && let ValueKind::Cast {
                value: inner,
                ty: JvmType::Int,
            } = ir.values[value].kind
        {
            value = inline_alias(ir, inner)?;
        }
    }
    let ValueKind::Binary { op, left, right } = ir.values[value].kind else {
        return None;
    };
    let ty = &ir.values[value].ty;
    if *target == JvmType::Boolean {
        if *ty != JvmType::Boolean
            || !matches!(op, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor)
            || ir.values[right]
                .local
                .map(|local| &ir.locals[local].ty)
                .unwrap_or(&ir.values[right].ty)
                != &JvmType::Boolean
        {
            return None;
        }
    } else if !numeric(ty) {
        return None;
    }
    let mut read = read_alias(ir, left);
    if let ValueKind::Cast {
        value,
        ty: promotion,
    } = &ir.values[read].kind
        && ir.values[read].local.is_none()
    {
        // A compound operation may perform this promotion itself. An earlier
        // narrowing of the old value is not interchangeable with that operation.
        if promotion != ty || !widening(target, promotion) {
            return None;
        }
        read = read_alias(ir, *value);
    }
    if matches!(op, BinaryOp::Shl | BinaryOp::Shr | BinaryOp::Ushr)
        && *ty != promoted(target, target)?
    {
        return None;
    }
    Some(Computation {
        op,
        read,
        right,
        ty,
    })
}

fn read_alias(ir: &MethodIr, mut value: ValueId) -> ValueId {
    for _ in 0..ir.values.len() {
        if ir.values[value].local.is_some() {
            break;
        }
        let ValueKind::Alias(inner) = ir.values[value].kind else {
            break;
        };
        value = inner;
    }
    value
}

/// A mapped read is a saved old value, even when its current source name looks
/// like the destination. Replacing it by a new lvalue read would change timing
/// and volatile access count. Only inline reads are eligible.
pub(crate) fn inline_alias(ir: &MethodIr, mut value: ValueId) -> Option<ValueId> {
    for _ in 0..ir.values.len() {
        if ir.values[value].local.is_some() {
            return None;
        }
        if let ValueKind::Alias(inner) = ir.values[value].kind {
            value = inner;
        } else {
            return Some(value);
        }
    }
    None
}
