//! Java 1.3 primitive conversions used at source expression boundaries.
use crate::classfile::JvmType;
use crate::ir::{BinaryOp, CompareOp, Literal, MethodIr, UnaryOp, ValueId, ValueKind};
use crate::program::Program;

/// Known types of the emitted Java expression, including null's bottom type.
/// Stored values and assignment expressions have their declaration's type;
/// an inline conditional may have a narrower type than its JVM int category.
/// If constantness is unresolved, `ty` is an upper bound and `types()` retains
/// the possible narrow types instead of claiming that the expression is int.
#[derive(Clone, Copy)]
pub(super) struct SourceType<'a> {
    pub ty: &'a JvmType,
    pub null: bool,
    alternatives: u8,
}

impl<'a> SourceType<'a> {
    pub fn value(ty: &'a JvmType) -> Self {
        Self {
            ty,
            null: false,
            alternatives: 0,
        }
    }

    pub fn types(self) -> impl Iterator<Item = &'a JvmType> {
        [
            Some(self.ty),
            (self.alternatives & 1 != 0).then_some(&JvmType::Byte),
            (self.alternatives & 2 != 0).then_some(&JvmType::Short),
            (self.alternatives & 4 != 0).then_some(&JvmType::Char),
        ]
        .into_iter()
        .flatten()
    }

    pub fn exactly(self, ty: &JvmType) -> bool {
        !self.null && self.alternatives == 0 && self.ty == ty
    }

    fn including(mut self, ty: &'a JvmType) -> Self {
        if self.ty == ty {
            return self;
        }
        let bit = |ty: &JvmType| match ty {
            JvmType::Byte => 1,
            JvmType::Short => 2,
            JvmType::Char => 4,
            _ => 0,
        };
        let upper = if widening(self.ty, ty) {
            ty
        } else if widening(ty, self.ty) {
            self.ty
        } else {
            &JvmType::Int
        };
        self.alternatives |= bit(self.ty) | bit(ty);
        self.alternatives &= !bit(upper);
        self.ty = upper;
        self
    }
}

pub(super) use crate::ir::type_facts::{numeric, promoted, widening};

pub(super) fn source_type(ir: &MethodIr, value: ValueId) -> &JvmType {
    ir.values[value]
        .local
        .map(|local| &ir.locals[local].ty)
        .unwrap_or(&ir.values[value].ty)
}

/// A materialized literal denotes its current local contents when read, not its
/// original initializer. Only an evaluated null definition has null's bottom
/// reference type and can rely on the surrounding assignment/choice context.
pub(super) fn null_literal(ir: &MethodIr, value: ValueId, root: bool) -> bool {
    (root || ir.values[value].local.is_none())
        && matches!(ir.values[value].kind, ValueKind::Literal(Literal::Null))
}

/// Java 1.3 conditional typing follows the emitted arms after their conversions.
/// Representable int constants and the byte/short pair are special cases before
/// ordinary binary numeric promotion. References are made explicit by the
/// expression planner, which retains its proven common reference type.
fn conditional_numeric_type<'a>(
    left: &'a JvmType,
    left_constant: Option<i32>,
    right: &'a JvmType,
    right_constant: Option<i32>,
) -> Option<&'a JvmType> {
    use JvmType::*;
    if !numeric(left) || !numeric(right) {
        return None;
    }
    if left == right {
        return Some(left);
    }
    if matches!((left, right), (Byte, Short) | (Short, Byte)) {
        return Some(&Short);
    }
    for (narrow, other, constant) in [(left, right, right_constant), (right, left, left_constant)] {
        if matches!(narrow, Byte | Short | Char)
            && *other == Int
            && constant.is_some_and(|value| {
                crate::ir::type_facts::IntegralRange::of_type(narrow)
                    .is_some_and(|range| range.min <= value && value <= range.max)
            })
        {
            return Some(narrow);
        }
    }
    Some(match promoted(left, right)? {
        Double => &Double,
        Float => &Float,
        Long => &Long,
        _ => &Int,
    })
}

/// Unknown constantness is not evidence that Java applies numeric promotion.
/// Keep all possible conditional types until a consumer can select a safe
/// descriptor conversion. This also covers source rewrites whose constantness
/// is not expressed by the original IR producer (for example concatenation).
pub(super) fn conditional_source_type<'a>(
    left: SourceType<'a>,
    left_constant: IntConstant,
    right: SourceType<'a>,
    right_constant: IntConstant,
) -> Option<SourceType<'a>> {
    let mut result: Option<SourceType<'a>> = None;
    let mut include = |ty| {
        result = Some(result.map_or(SourceType::value(ty), |known| known.including(ty)));
    };
    for left in left.types() {
        for right in right.types() {
            let ty = conditional_numeric_type(
                left,
                left_constant.known(),
                right,
                right_constant.known(),
            )?;
            include(ty);
            for (narrow, other, constant) in
                [(left, right, right_constant), (right, left, left_constant)]
            {
                if *other == JvmType::Int
                    && matches!(narrow, JvmType::Byte | JvmType::Short | JvmType::Char)
                    && constant == IntConstant::Unknown
                {
                    include(narrow);
                }
            }
        }
    }
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IntConstant {
    Known(i32),
    NotConstant,
    Unknown,
}

impl IntConstant {
    fn known(self) -> Option<i32> {
        match self {
            Self::Known(value) => Some(value),
            _ => None,
        }
    }
}

/// The representable-constant rule uses Java constant expressions, not value
/// ranges. In particular a mutable local initialized from 0 is not a constant,
/// and an assignment expression remains nonconstant even when its RHS is one.
pub(super) fn int_constant(program: &Program<'_>, ir: &MethodIr, value: ValueId) -> IntConstant {
    match constant(program, ir, value, 0) {
        Some(Number::Int(value)) => IntConstant::Known(value),
        _ if nonconstant(program, ir, value, 0) => IntConstant::NotConstant,
        _ => IntConstant::Unknown,
    }
}

/// This is only a negative proof over syntax that survives rendering. Unknown
/// producers stay unknown; notably a String invocation may become concatenation
/// and a final String field can participate in a Java constant expression.
fn nonconstant(program: &Program<'_>, ir: &MethodIr, value: ValueId, depth: usize) -> bool {
    if ir.values[value].local.is_some() {
        return true;
    }
    if depth >= 512 {
        return false;
    }
    let recurse = |value| nonconstant(program, ir, value, depth + 1);
    match &ir.values[value].kind {
        ValueKind::Parameter { .. }
        | ValueKind::InlineAssign { .. }
        | ValueKind::LocalUpdate { .. }
        | ValueKind::Update { .. }
        | ValueKind::CompoundUpdate { .. }
        | ValueKind::ArrayLoad { .. }
        | ValueKind::ArrayLength { .. }
        | ValueKind::New { .. }
        | ValueKind::NewArray { .. }
        | ValueKind::InstanceOf { .. }
        | ValueKind::Literal(Literal::Null | Literal::Class(_)) => true,
        ValueKind::Call { .. } => !ir.values[value].ty.is_reference(),
        ValueKind::Field { field, .. } => program
            .resolve_field(&field.owner, &field.name, &field.descriptor)
            .is_some_and(|member| {
                // A final field without ConstantValue can acquire a constant
                // initializer during source reconstruction (notably interfaces).
                // Missing input metadata does not prove nonconstant Java syntax.
                matches!(
                    super::fields::constant_field(program, member),
                    super::fields::ConstantField::Nonconstant
                )
            }),
        ValueKind::Alias(value)
        | ValueKind::Cast { value, .. }
        | ValueKind::Unary { value, .. } => recurse(*value),
        ValueKind::Binary { left, right, .. } => recurse(*left) || recurse(*right),
        ValueKind::Select {
            condition,
            then_value,
            else_value,
        } => [condition.left, condition.right, *then_value, *else_value]
            .into_iter()
            .any(recurse),
        _ => false,
    }
}

#[derive(Clone, Copy)]
enum Number {
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
}

impl Number {
    fn int(self) -> i32 {
        match self {
            Self::Int(value) => value,
            Self::Long(value) => value as i32,
            Self::Float(value) => value as i32,
            Self::Double(value) => value as i32,
        }
    }

    fn long(self) -> i64 {
        match self {
            Self::Int(value) => value.into(),
            Self::Long(value) => value,
            Self::Float(value) => value as i64,
            Self::Double(value) => value as i64,
        }
    }

    fn float(self) -> f32 {
        match self {
            Self::Int(value) => value as f32,
            Self::Long(value) => value as f32,
            Self::Float(value) => value,
            Self::Double(value) => value as f32,
        }
    }

    fn double(self) -> f64 {
        match self {
            Self::Int(value) => value.into(),
            Self::Long(value) => value as f64,
            Self::Float(value) => value.into(),
            Self::Double(value) => value,
        }
    }

    fn cast(self, ty: &JvmType) -> Option<Self> {
        Some(match ty {
            JvmType::Boolean => Self::Int(i32::from(self.int() != 0)),
            JvmType::Byte => Self::Int(self.int() as i8 as i32),
            JvmType::Short => Self::Int(self.int() as i16 as i32),
            JvmType::Char => Self::Int(self.int() as u16 as i32),
            JvmType::Int => Self::Int(self.int()),
            JvmType::Long => Self::Long(self.long()),
            JvmType::Float => Self::Float(self.float()),
            JvmType::Double => Self::Double(self.double()),
            _ => return None,
        })
    }

    fn binary(self, op: BinaryOp, right: Self, ty: &JvmType) -> Option<Self> {
        use BinaryOp::*;
        macro_rules! integral {
            ($left:expr, $right:expr, $unsigned:ty) => {{
                let (left, right) = ($left, $right);
                match op {
                    Add => left.wrapping_add(right),
                    Sub => left.wrapping_sub(right),
                    Mul => left.wrapping_mul(right),
                    Div if right != 0 => left.wrapping_div(right),
                    Rem if right != 0 => left.wrapping_rem(right),
                    Shl => left.wrapping_shl(right as u32),
                    Shr => left.wrapping_shr(right as u32),
                    Ushr => (left as $unsigned).wrapping_shr(right as u32) as _,
                    And => left & right,
                    Or => left | right,
                    Xor => left ^ right,
                    _ => return None,
                }
            }};
        }
        macro_rules! floating {
            ($left:expr, $right:expr) => {{
                let (left, right) = ($left, $right);
                match op {
                    Add => left + right,
                    Sub => left - right,
                    Mul => left * right,
                    Div => left / right,
                    Rem => left % right,
                    _ => return None,
                }
            }};
        }
        Some(match ty {
            JvmType::Long => Self::Long(integral!(self.long(), right.long(), u64)),
            JvmType::Float => Self::Float(floating!(self.float(), right.float())),
            JvmType::Double => Self::Double(floating!(self.double(), right.double())),
            ty if ty.is_int_like() => Self::Int(integral!(self.int(), right.int(), u32)),
            _ => return None,
        })
    }

    fn compare(self, op: CompareOp, right: Self) -> bool {
        let order = match (self, right) {
            (Self::Double(_), _) | (_, Self::Double(_)) => {
                self.double().partial_cmp(&right.double())
            }
            (Self::Float(_), _) | (_, Self::Float(_)) => self.float().partial_cmp(&right.float()),
            (Self::Long(_), _) | (_, Self::Long(_)) => Some(self.long().cmp(&right.long())),
            _ => Some(self.int().cmp(&right.int())),
        };
        op.test(order)
    }
}

fn constant(program: &Program<'_>, ir: &MethodIr, value: ValueId, depth: usize) -> Option<Number> {
    if depth >= 512 || ir.values[value].local.is_some() {
        return None;
    }
    let definition = &ir.values[value];
    let recurse = |value| constant(program, ir, value, depth + 1);
    match &definition.kind {
        ValueKind::Literal(Literal::Int(value)) => Some(Number::Int(*value)),
        ValueKind::Literal(Literal::Long(value)) => Some(Number::Long(*value)),
        ValueKind::Literal(Literal::Float(value)) => Some(Number::Float(*value)),
        ValueKind::Literal(Literal::Double(value)) => Some(Number::Double(*value)),
        ValueKind::Alias(value) => recurse(*value),
        ValueKind::Cast { value, ty } => recurse(*value)?.cast(ty),
        ValueKind::Unary {
            op: UnaryOp::Neg,
            value,
        } => Some(match recurse(*value)? {
            Number::Int(value) => Number::Int(value.wrapping_neg()),
            Number::Long(value) => Number::Long(value.wrapping_neg()),
            Number::Float(value) => Number::Float(-value),
            Number::Double(value) => Number::Double(-value),
        }),
        ValueKind::Binary { op, left, right } => {
            recurse(*left)?.binary(*op, recurse(*right)?, &definition.ty)
        }
        ValueKind::Select {
            condition,
            then_value,
            else_value,
        } => {
            let left = recurse(condition.left)?;
            let right = recurse(condition.right)?;
            // Both arms must be constant expressions, even when the condition
            // determines that only one of their values can be selected.
            let yes = recurse(*then_value)?;
            let no = recurse(*else_value)?;
            if left.compare(condition.op, right) {
                yes
            } else {
                no
            }
            .cast(&definition.ty)
        }
        ValueKind::Field {
            field,
            receiver: None,
        } => {
            let member = program.resolve_field(&field.owner, &field.name, &field.descriptor)?;
            let super::fields::ConstantField::Known(constant) =
                super::fields::constant_field(program, member)
            else {
                return None;
            };
            use crate::classfile::Constant;
            let constant = program
                .class(member.class)
                .constant_pool
                .get(constant)
                .ok()?;
            let number = match constant {
                Constant::Integer(value) => Number::Int(*value),
                Constant::Long(value) => Number::Long(*value),
                Constant::Float(value) => Number::Float(*value),
                Constant::Double(value) => Number::Double(*value),
                _ => return None,
            };
            number.cast(&definition.ty)
        }
        _ => None,
    }
}

/// Remove only the outer conversion which the surrounding context will perform
/// itself. Intermediate narrowing or floating-point rounding stays explicit.
pub(super) fn widened_operand(
    ir: &MethodIr,
    value: ValueId,
    expected: &JvmType,
    root: bool,
) -> Option<ValueId> {
    if !root && ir.values[value].local.is_some() {
        return None;
    }
    let ValueKind::Cast { value, ty } = &ir.values[value].kind else {
        return None;
    };
    (ty == expected && numeric(ty) && widening(source_type(ir, *value), ty)).then_some(*value)
}

/// Select operands together: dropping both widening casts independently can
/// change an operation back to int arithmetic and introduce overflow/division.
pub(super) fn binary_operands(
    ir: &MethodIr,
    left: ValueId,
    right: ValueId,
    ty: &JvmType,
) -> (ValueId, ValueId) {
    let mut left = left;
    let mut right = right;
    if let Some(inner) = widened_operand(ir, right, ty, false)
        && promoted(source_type(ir, left), source_type(ir, inner)).as_ref() == Some(ty)
    {
        right = inner;
    }
    if let Some(inner) = widened_operand(ir, left, ty, false)
        && promoted(source_type(ir, inner), source_type(ir, right)).as_ref() == Some(ty)
    {
        left = inner;
    }
    (left, right)
}

#[cfg(test)]
#[path = "../../tests/unit/java_conversions.rs"]
mod tests;
