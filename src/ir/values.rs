use super::*;

impl MethodIr {
    /// Whether the operation itself constrains source evaluation order. Operand
    /// effects are separate: division by a known nonzero constant and floating
    /// point division are pure, even when evaluating their numerator is not.
    pub(crate) fn operation_is_observable(&self, id: ValueId) -> bool {
        let value = &self.values[id];
        if let ValueKind::Binary {
            op: BinaryOp::Div | BinaryOp::Rem,
            right,
            ..
        } = value.kind
        {
            return matches!(value.ty, JvmType::Int | JvmType::Long)
                && !matches!(self.values[right].kind,
                    ValueKind::Literal(Literal::Int(value)) if value != 0)
                && !matches!(self.values[right].kind,
                    ValueKind::Literal(Literal::Long(value)) if value != 0);
        }
        value.kind.is_observable()
    }
}

impl Value {
    /// Whether evaluating this operation itself can transfer to a handler.
    /// Callers inspecting a source expression also visit its evaluated operands;
    /// loading an already materialized value is handled by the caller.
    pub fn may_throw(&self) -> bool {
        match &self.kind {
            // Literal/type resolution only adds linkage or VM-exhaustion
            // failures, outside the source equivalence model. Treating reused
            // literals as new throwing operations also invents catch edges.
            ValueKind::InlineAssign { .. }
            | ValueKind::LocalUpdate { .. }
            | ValueKind::Literal(_)
            | ValueKind::InstanceOf { .. } => false,
            ValueKind::Binary {
                op: BinaryOp::Div | BinaryOp::Rem,
                ..
            } => matches!(self.ty, JvmType::Int | JvmType::Long),
            other => other.is_observable(),
        }
    }
}

impl ValueKind {
    /// Embedded local writes share their storage effect, independently of
    /// whether the expression yields the committed RHS or the previous value.
    pub(crate) fn local_write(&self) -> Option<(LocalId, ValueId)> {
        match self {
            Self::InlineAssign { local, value } | Self::LocalUpdate { local, value, .. } => {
                Some((*local, *value))
            }
            _ => None,
        }
    }

    pub(crate) fn local_write_mut(&mut self) -> Option<(&mut LocalId, &mut ValueId)> {
        match self {
            Self::InlineAssign { local, value } | Self::LocalUpdate { local, value, .. } => {
                Some((local, value))
            }
            _ => None,
        }
    }

    pub fn operands(&self) -> Vec<ValueId> {
        let mut result = Vec::new();
        self.for_each_operand(|value| result.push(value));
        result
    }

    pub fn for_each_operand(&self, mut visit: impl FnMut(ValueId)) {
        match self {
            Self::Select {
                condition,
                then_value,
                else_value,
            } => {
                visit(condition.left);
                visit(condition.right);
                visit(*then_value);
                visit(*else_value);
            }
            Self::Phi { inputs, .. } => {
                for input in inputs {
                    visit(input.value);
                }
            }
            Self::Alias(value)
            | Self::InlineAssign { value, .. }
            | Self::LocalUpdate { value, .. }
            | Self::Unary { value, .. }
            | Self::Cast { value, .. }
            | Self::InstanceOf { value, .. } => visit(*value),
            Self::Binary { left, right, .. } | Self::Compare { left, right, .. } => {
                visit(*left);
                visit(*right);
            }
            Self::Field {
                receiver: Some(value),
                ..
            } => visit(*value),
            Self::Update { target, .. } => target.for_each_operand(&mut visit),
            Self::CompoundUpdate { target, right, .. } => {
                target.for_each_operand(&mut visit);
                visit(*right);
            }
            Self::ArrayLoad { array, index, .. } => {
                visit(*array);
                visit(*index);
            }
            Self::ArrayLength { array } => visit(*array),
            Self::Call { receiver, args, .. } => {
                if let Some(value) = receiver {
                    visit(*value);
                }
                for value in args {
                    visit(*value);
                }
            }
            Self::New { args, .. } => {
                for value in args {
                    visit(*value);
                }
            }
            Self::NewArray { dimensions, .. } => {
                for value in dimensions {
                    visit(*value);
                }
            }
            Self::ArrayInitializer { values, .. } => {
                for value in values {
                    visit(*value);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn map_operands(&mut self, mut map: impl FnMut(ValueId) -> ValueId) {
        match self {
            Self::Select {
                condition,
                then_value,
                else_value,
            } => {
                condition.left = map(condition.left);
                condition.right = map(condition.right);
                *then_value = map(*then_value);
                *else_value = map(*else_value);
            }
            Self::Phi { inputs, .. } => {
                for input in inputs {
                    input.value = map(input.value);
                }
            }
            Self::Alias(value)
            | Self::InlineAssign { value, .. }
            | Self::LocalUpdate { value, .. }
            | Self::Unary { value, .. }
            | Self::Cast { value, .. }
            | Self::InstanceOf { value, .. } => *value = map(*value),
            Self::Binary { left, right, .. } | Self::Compare { left, right, .. } => {
                *left = map(*left);
                *right = map(*right);
            }
            Self::Field {
                receiver: Some(value),
                ..
            } => *value = map(*value),
            Self::Update { target, .. } => target.map_operands(&mut map),
            Self::CompoundUpdate { target, right, .. } => {
                target.map_operands(&mut map);
                *right = map(*right);
            }
            Self::ArrayLoad { array, index, .. } => {
                *array = map(*array);
                *index = map(*index);
            }
            Self::ArrayLength { array } => *array = map(*array),
            Self::Call { receiver, args, .. } => {
                if let Some(value) = receiver {
                    *value = map(*value);
                }
                for value in args {
                    *value = map(*value);
                }
            }
            Self::New { args, .. } => {
                for value in args {
                    *value = map(*value);
                }
            }
            Self::NewArray { dimensions, .. } => {
                for value in dimensions {
                    *value = map(*value);
                }
            }
            Self::ArrayInitializer { values, .. } => {
                for value in values {
                    *value = map(*value);
                }
            }
            _ => {}
        }
    }

    /// Operations whose evaluation can observe or change runtime state. Reads
    /// are included: evaluating the same field/array access twice is not a copy.
    pub fn is_observable(&self) -> bool {
        matches!(
            self,
            Self::InitializeClass { .. }
                | Self::Field { .. }
                | Self::Update { .. }
                | Self::CompoundUpdate { .. }
                | Self::InlineAssign { .. }
                | Self::LocalUpdate { .. }
                | Self::ArrayLoad { .. }
                | Self::ArrayLength { .. }
                | Self::Call { .. }
                | Self::New { .. }
                | Self::NewArray { .. }
                | Self::ArrayInitializer { .. }
                | Self::Allocation { .. }
                | Self::Cast {
                    ty: JvmType::Object(_) | JvmType::Array(_),
                    ..
                }
                | Self::Binary {
                    op: BinaryOp::Div | BinaryOp::Rem,
                    ..
                }
        )
    }
}

impl Statement {
    pub fn operands(&self) -> Vec<ValueId> {
        match self {
            Self::Assign { value, .. } => vec![value.value()],
            Self::Eval(value) | Self::Monitor { value, .. } => {
                vec![*value]
            }
            Self::PutField {
                receiver, value, ..
            } => receiver.iter().copied().chain([*value]).collect(),
            Self::ArrayStore {
                array,
                index,
                value,
                ..
            } => vec![*array, *index, *value],
            Self::ConstructorCall { receiver, args, .. } => std::iter::once(*receiver)
                .chain(args.iter().copied())
                .collect(),
        }
    }

    pub(crate) fn map_operands(&mut self, mut map: impl FnMut(ValueId) -> ValueId) {
        match self {
            Self::Assign { value, .. } => *value = value.map_value(map),
            Self::Eval(value) | Self::Monitor { value, .. } => *value = map(*value),
            Self::PutField {
                receiver, value, ..
            } => {
                if let Some(receiver) = receiver {
                    *receiver = map(*receiver);
                }
                *value = map(*value);
            }
            Self::ArrayStore {
                array,
                index,
                value,
                ..
            } => {
                *array = map(*array);
                *index = map(*index);
                *value = map(*value);
            }
            Self::ConstructorCall { receiver, args, .. } => {
                *receiver = map(*receiver);
                for value in args {
                    *value = map(*value);
                }
            }
        }
    }
}

impl Terminator {
    pub fn operands(&self) -> Vec<ValueId> {
        match self {
            Self::Branch { condition, .. } => vec![condition.left, condition.right],
            Self::Switch { value, .. } | Self::Throw { value, .. } | Self::Return(Some(value)) => {
                vec![*value]
            }
            _ => vec![],
        }
    }

    pub(crate) fn map_operands(&mut self, mut map: impl FnMut(ValueId) -> ValueId) {
        match self {
            Self::Branch { condition, .. } => {
                condition.left = map(condition.left);
                condition.right = map(condition.right);
            }
            Self::Switch { value, .. } | Self::Throw { value, .. } | Self::Return(Some(value)) => {
                *value = map(*value)
            }
            _ => {}
        }
    }

    pub(crate) fn redirect(&mut self, from: BlockId, to: BlockId) {
        match self {
            Self::Goto(target) => {
                if *target == from {
                    *target = to;
                }
            }
            Self::Branch {
                then_target,
                else_target,
                ..
            } => {
                if *then_target == from {
                    *then_target = to;
                }
                if *else_target == from {
                    *else_target = to;
                }
            }
            Self::Switch { cases, default, .. } => {
                if *default == from {
                    *default = to;
                }
                for (_, target) in cases {
                    if *target == from {
                        *target = to;
                    }
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn join_type(left: &JvmType, right: &JvmType) -> JvmType {
    use JvmType::*;
    if left == right {
        return left.clone();
    }
    if is_integral(left) && is_integral(right) {
        return Int;
    }
    match (left, right) {
        (Array(a), Array(b)) if a.is_reference() && b.is_reference() => {
            Array(Box::new(join_type(a, b)))
        }
        _ if left.is_reference() && right.is_reference() => Object("java/lang/Object".into()),
        _ => left.clone(),
    }
}

pub(crate) fn is_integral(ty: &JvmType) -> bool {
    matches!(
        ty,
        JvmType::Boolean | JvmType::Byte | JvmType::Char | JvmType::Short | JvmType::Int
    )
}

impl UpdateTarget {
    pub(crate) fn for_each_operand(&self, mut visit: impl FnMut(ValueId)) {
        match self {
            Self::Field { receiver, .. } => receiver.iter().copied().for_each(visit),
            Self::Array { array, index, .. } => {
                visit(*array);
                visit(*index);
            }
        }
    }

    pub(crate) fn map_operands(&mut self, mut map: impl FnMut(ValueId) -> ValueId) {
        match self {
            Self::Field { receiver, .. } => *receiver = receiver.map(map),
            Self::Array { array, index, .. } => {
                *array = map(*array);
                *index = map(*index);
            }
        }
    }
}
