//! Normalize bound values and their consumer conversions through one expression
//! pipeline. Body cleanup and rewritten statements supply the surrounding context.
use super::bound::{AssignmentValue, Body, Simple, Statement};
use super::evaluation::{Effects, Evaluation, children, effects, evaluations, simple};
use crate::{
    classfile::JvmType,
    java::syntax::{Expr, Expression, Operator, Prefix},
};

pub(super) fn statement(statement: &mut Statement, result: &super::bound::ValueType) {
    normalize(statement, result, true);
}

fn normalize(statement: &mut Statement, result: &super::bound::ValueType, tests: bool) {
    evaluations(statement, &mut Normalize);
    match statement {
        Statement::If { condition, .. } if tests => normalize_test(condition),
        Statement::Loop { header, .. } if tests => {
            let (super::LoopHeader::While(condition)
            | super::LoopHeader::DoWhile(condition)
            | super::LoopHeader::For { condition, .. }) = header;
            normalize_test(condition);
        }
        _ => (),
    }
    normalize_result(statement, result);
}

/// A truth consumer observes nonzero, unlike a stored Boolean or equality
/// operand. Propagate that context only through truth-preserving constructs;
/// eager bitwise operands and assignments must retain their original bits.
pub(super) fn normalize_test(value: &mut Expr) {
    if let Expression::Prefix(crate::java::syntax::Prefix::Not, inner) = &value.kind
        && let Expression::Prefix(crate::java::syntax::Prefix::Not, original) = &inner.kind
    {
        *value = *original.clone();
        normalize_test(value);
        return;
    }
    if let Expression::Conditional(arms) = &value.kind
        && let (Some(yes), Some(no)) = (arms[1].boolean_constant(), arms[2].boolean_constant())
        && yes != no
    {
        *value = if yes {
            arms[0].clone()
        } else {
            arms[0].clone().negated_test()
        };
        normalize_test(value);
        return;
    }
    match &mut value.kind {
        Expression::Logical { parts, .. } => {
            for part in parts {
                normalize_test(part);
            }
        }
        Expression::Conditional(arms) => {
            for arm in arms.iter_mut() {
                normalize_test(arm);
            }
            value.facts.canonical_boolean =
                arms[1].canonical_boolean() && arms[2].canonical_boolean();
        }
        Expression::Group(inner) | Expression::Commented(inner, _) => {
            normalize_test(inner);
            value.facts.canonical_boolean = inner.canonical_boolean();
        }
        _ => (),
    }
}

struct Normalize;
impl Evaluation for Normalize {
    fn finish(&mut self, expression: &mut Expr) {
        // These operands are truth tests even when their parent's result is
        // assigned, returned, or compared. A previous value projection can
        // therefore be discharged here without changing its raw-bit siblings.
        match &mut expression.kind {
            Expression::Logical { parts, .. } => {
                for part in parts {
                    normalize_test(part);
                }
            }
            Expression::Prefix(crate::java::syntax::Prefix::Not, value) => normalize_test(value),
            Expression::Conditional(arms) => normalize_test(&mut arms[0]),
            _ => (),
        }
        if let Expression::Prefix(crate::java::syntax::Prefix::Not, value) = &expression.kind {
            *expression = Expr::prefix(crate::java::syntax::Prefix::Not, *value.clone());
        }
        if let Expression::Prefix(op @ (Prefix::Negate | Prefix::Complement), value) =
            &expression.kind
            && let Some(ty @ (JvmType::Int | JvmType::Long)) = expression.facts.ty.as_ref()
            && value.facts.ty.as_ref() == Some(ty)
            && matches!(&value.kind, Expression::Prefix(inner, _) if matches!(
                (op, inner),
                (Prefix::Negate, Prefix::Negate) | (Prefix::Complement, Prefix::Complement)
            ))
        {
            *expression = integral_prefix(*op, value, ty);
        }
        if let Some(value) = guarded_choice(expression) {
            *expression = value;
        } else if let Some(value) = bitwise_identity(expression) {
            *expression = value;
        } else if let Some(value) = additive_identity(expression) {
            *expression = value;
        } else if let Expression::Conditional(arms) = &mut expression.kind
            && same_value(&arms[1], &arms[2])
            && effects(&mut arms[0]).discardable()
        {
            // Equal outcomes do not erase a throwing or effectful selector.
            // Keep the expression's original apparent Java type.
            let ty = expression.facts.ty.clone();
            *expression = preserve_type(arms[1].clone(), ty);
        }
        normalize_conversions(expression);
    }
}

fn integral_width(ty: &JvmType) -> Option<u8> {
    Some(match ty {
        JvmType::Byte => 8,
        JvmType::Short | JvmType::Char => 16,
        JvmType::Int => 32,
        JvmType::Long => 64,
        _ => return None,
    })
}

fn int_constant(value: &Expr) -> crate::java::conversions::IntConstant {
    use crate::java::conversions::IntConstant;
    if let Some(value) = value
        .integer_value()
        .and_then(|value| i32::try_from(value).ok())
    {
        return IntConstant::Known(value);
    }
    match &value.kind {
        Expression::Group(value) | Expression::Commented(value, _) => int_constant(value),
        Expression::Cast {
            value: inner,
            primitive: true,
            ..
        } => match int_constant(inner) {
            IntConstant::Known(constant) => match value.source_type() {
                Some(JvmType::Byte) => IntConstant::Known(i32::from(constant as i8)),
                Some(JvmType::Short) => IntConstant::Known(i32::from(constant as i16)),
                Some(JvmType::Char) => IntConstant::Known(i32::from(constant as u16)),
                Some(JvmType::Int | JvmType::Long) => IntConstant::Known(constant),
                _ => IntConstant::Unknown,
            },
            constant => constant,
        },
        Expression::Local { .. }
        | Expression::Assignment { .. }
        | Expression::Update { .. }
        | Expression::Invocation { .. }
        | Expression::Index(_) => IntConstant::NotConstant,
        _ => IntConstant::Unknown,
    }
}

fn representable(constant: crate::java::conversions::IntConstant, ty: &JvmType) -> bool {
    let crate::java::conversions::IntConstant::Known(value) = constant else {
        return false;
    };
    crate::ir::type_facts::IntegralRange::of_type(ty)
        .is_some_and(|range| range.min <= value && value <= range.max)
}

fn assignment_conversion(
    ty: &JvmType,
    constant: crate::java::conversions::IntConstant,
    target: &JvmType,
) -> bool {
    crate::java::conversions::widening(ty, target)
        || matches!(
            ty,
            JvmType::Byte | JvmType::Short | JvmType::Char | JvmType::Int
        ) && representable(constant, target)
}

fn conditional_type(
    left: &JvmType,
    left_constant: crate::java::conversions::IntConstant,
    right: &JvmType,
    right_constant: crate::java::conversions::IntConstant,
) -> Option<JvmType> {
    use crate::java::conversions::{SourceType, conditional_source_type};
    let ty = conditional_source_type(
        SourceType::value(left),
        left_constant,
        SourceType::value(right),
        right_constant,
    )?;
    ty.exactly(ty.ty).then(|| ty.ty.clone())
}

/// A substitution retains its apparent type until its actual consumer is known.
/// Discharge only value-preserving integral casts; calls and concatenation keep
/// exact types, while stores, arithmetic and choices supply their Java context.
fn contextual_conversion(
    value: &mut Expr,
    accepts: &dyn Fn(&JvmType, crate::java::conversions::IntConstant) -> bool,
) {
    while let Expression::Cast {
        primitive: true,
        value: inner,
        ..
    } = &mut value.kind
    {
        let (Some(from), Some(to)) = (inner.source_type(), value.facts.ty.as_ref()) else {
            break;
        };
        let lossless = from == to
            || integral_width(from).is_some()
                && integral_width(to).is_some()
                && (crate::java::conversions::widening(from, to)
                    || representable(int_constant(inner), to));
        if !lossless || !accepts(from, int_constant(inner)) {
            break;
        }
        *value = std::mem::replace(inner.as_mut(), Expr::boolean(false));
    }
    match &mut value.kind {
        Expression::Group(inner) | Expression::Commented(inner, _) => {
            contextual_conversion(inner, accepts);
            value.facts.ty = inner.facts.ty.clone();
            value.facts.integer_literal = inner.facts.integer_literal;
        }
        Expression::Conditional(arms)
            if value.facts.ty.as_ref().and_then(integral_width).is_some() =>
        {
            for index in [1, 2] {
                let other = &arms[3 - index];
                let Some(other_ty) = other.source_type().cloned() else {
                    continue;
                };
                let other_constant = int_constant(other);
                contextual_conversion(&mut arms[index], &|ty, constant| {
                    conditional_type(ty, constant, &other_ty, other_constant).is_some_and(|ty| {
                        integral_width(&ty).is_some()
                            && accepts(&ty, crate::java::conversions::IntConstant::Unknown)
                    })
                });
            }
            if let (Some(left), Some(right)) = (arms[1].source_type(), arms[2].source_type())
                && let Some(ty) =
                    conditional_type(left, int_constant(&arms[1]), right, int_constant(&arms[2]))
            {
                value.facts.ty = Some(ty);
            }
        }
        _ => (),
    }
}

fn assigned_conversion(value: &mut Expr, target: &JvmType) {
    if target.is_reference()
        && let Expression::Cast {
            primitive: false,
            value: inner,
            ..
        } = &value.kind
        && inner.is_null()
    {
        *value = *inner.clone();
    }
    contextual_conversion(value, &|ty, constant| {
        assignment_conversion(ty, constant, target)
    });
}

fn normalize_conversions(expression: &mut Expr) {
    use crate::java::conversions::{numeric, promoted, widening};
    let original = expression.facts.ty.clone();
    match &mut expression.kind {
        Expression::Assignment {
            operands,
            compound: None,
        } => {
            if let Some(target) = operands[0].source_type().cloned() {
                assigned_conversion(&mut operands[1], &target);
            }
        }
        Expression::Cast {
            primitive: true,
            value,
            ..
        } => {
            if let Some(target) = &original {
                let value = value.as_mut();
                while let Expression::Cast {
                    primitive: true,
                    value: inner,
                    ..
                } = &mut value.kind
                {
                    let (Some(from), Some(via)) = (inner.source_type(), value.facts.ty.as_ref())
                    else {
                        break;
                    };
                    // Direct casts to byte/short/char already include Java's
                    // int step, including saturation of floating-point inputs.
                    let int_step = *via == JvmType::Int
                        && matches!(target, JvmType::Byte | JvmType::Short | JvmType::Char)
                        && numeric(from);
                    let integral = match (
                        integral_width(from),
                        integral_width(via),
                        integral_width(target),
                    ) {
                        (Some(_), Some(via_width), Some(width)) => {
                            widening(from, via) || width <= via_width
                        }
                        _ => false,
                    };
                    if from != via && via != target && !int_step && !integral {
                        break;
                    }
                    *value = std::mem::replace(inner.as_mut(), Expr::boolean(false));
                }
                contextual_conversion(value, &|ty, _| numeric(ty));
            }
        }
        Expression::Binary(operator, operands) => {
            let shift = matches!(
                operator,
                Operator::ShiftLeft | Operator::ShiftRight | Operator::UnsignedShiftRight
            );
            for index in [0, 1] {
                let Some(other) = operands[1 - index].source_type().cloned() else {
                    continue;
                };
                let Some(before) = operands[index].source_type().cloned() else {
                    continue;
                };
                let expected = promoted(&before, &other);
                contextual_conversion(&mut operands[index], &|ty, _| {
                    if shift {
                        integral_width(ty).is_some()
                            && (index == 1 || (*ty == JvmType::Long) == (before == JvmType::Long))
                    } else {
                        expected.is_some() && promoted(ty, &other) == expected
                    }
                });
            }
        }
        Expression::ArrayInitializer { elements, .. } => {
            if let Some(JvmType::Array(element)) = &original {
                for value in elements {
                    assigned_conversion(value, element);
                }
            }
        }
        _ => (),
    }
    if let Some(ty) = original {
        contextual_conversion(expression, &|candidate, _| candidate == &ty);
    }
}

fn normalize_result(statement: &mut Statement, result: &super::bound::ValueType) {
    let simple = |value: &mut Simple| {
        if let Simple::Assign {
            binding,
            value: AssignmentValue::Value(value),
            ..
        } = value
        {
            assigned_conversion(value, &binding.ty);
        }
    };
    match statement {
        Statement::Return(Some(value)) => assigned_conversion(value, &result.ty),
        Statement::Simple(value) => simple(value),
        Statement::Loop {
            header:
                super::LoopHeader::For {
                    initializer,
                    update,
                    ..
                },
            ..
        } => {
            if let Some(initializer) = initializer {
                simple(initializer);
            }
            simple(update);
        }
        _ => (),
    }
}

/// A stable prerequisite can choose the entire result. Path facts retain the
/// distinction between a false slot (exactly zero) and a true raw slot (merely
/// nonzero), so eager masks keep their original bits and evaluations.
fn guarded_choice(expression: &Expr) -> Option<Expr> {
    let Expression::Logical { and: false, parts } = &expression.kind else {
        return None;
    };
    let [first, alternative] = parts.as_slice() else {
        return None;
    };
    let Expression::Logical { and: true, parts } = &first.kind else {
        return None;
    };
    let (guard, rest) = parts.split_first()?;
    if rest.is_empty() {
        return None;
    }
    let mut guard = guard.clone();
    let guard_effects = effects(&mut guard);
    if !guard_effects.discardable() {
        return None;
    }
    let mut selected = if rest.len() == 1 {
        rest[0].clone()
    } else {
        Expr::logical(true, rest.to_vec())
    };
    let selected_effects = effects(&mut selected);
    if selected_effects
        .writes
        .keys()
        .any(|local| guard_effects.reads.contains_key(local))
    {
        return None;
    }
    if effects(&mut alternative.clone())
        .writes
        .keys()
        .any(|local| guard_effects.reads.contains_key(local))
    {
        return None;
    }
    let when_selected = assumed(alternative.clone(), &guard, true, true);
    if when_selected.boolean_constant() != Some(false) {
        return None;
    }
    let other = assumed(alternative.clone(), &guard, false, true);
    Some(Expr::conditional(
        guard,
        selected.truth_value(),
        other.truth_value(),
    ))
}

fn known_test(value: &Expr, guard: &Expr, truth: bool) -> Option<bool> {
    if same_value(value, guard) {
        Some(truth)
    } else if same_value(value, &guard.clone().negated_test()) {
        Some(!truth)
    } else {
        None
    }
}

fn assumed(mut value: Expr, guard: &Expr, truth: bool, test: bool) -> Expr {
    if let Some(known) = known_test(&value, guard, truth)
        && (test || !known || value.canonical_boolean())
    {
        return Expr::boolean(known);
    }
    match &mut value.kind {
        Expression::Prefix(crate::java::syntax::Prefix::Not, inner) => {
            let inner = assumed(*inner.clone(), guard, truth, true);
            if let Some(constant) = inner.boolean_constant() {
                Expr::boolean(!constant)
            } else {
                Expr::prefix(crate::java::syntax::Prefix::Not, inner)
            }
        }
        Expression::Group(inner) => assumed(*inner.clone(), guard, truth, test).grouped(),
        Expression::Commented(inner, comment) => {
            assumed(*inner.clone(), guard, truth, test).commented(comment.clone())
        }
        Expression::Logical { and, parts } => {
            let mut kept = Vec::new();
            for part in std::mem::take(parts) {
                let part = assumed(part, guard, truth, true);
                if let Some(constant) = part.boolean_constant() {
                    if constant == *and {
                        continue;
                    }
                    if kept.is_empty() {
                        return Expr::boolean(constant);
                    }
                    kept.push(part);
                    break;
                }
                kept.push(part);
            }
            match kept.len() {
                0 => Expr::boolean(*and),
                1 if test => kept.pop().unwrap(),
                1 => kept.pop().unwrap().truth_value(),
                _ => Expr::logical(*and, kept),
            }
        }
        Expression::Binary(operator @ (Operator::And | Operator::Or | Operator::Xor), operands)
            if value.facts.ty == Some(JvmType::Boolean) =>
        {
            let left = assumed(operands[0].clone(), guard, truth, false);
            let right = assumed(operands[1].clone(), guard, truth, false);
            for (constant, other) in [(&left, &right), (&right, &left)] {
                if let Some(constant) = constant.boolean_constant() {
                    let mut other = other.clone();
                    match (*operator, constant) {
                        (Operator::And, false) if effects(&mut other).discardable() => {
                            return Expr::boolean(false);
                        }
                        (Operator::And, true) if other.canonical_boolean() => return other,
                        (Operator::Or | Operator::Xor, false) => return other,
                        (Operator::Or, true)
                            if other.canonical_boolean() && effects(&mut other).discardable() =>
                        {
                            return Expr::boolean(true);
                        }
                        (Operator::Xor, true) if other.canonical_boolean() => {
                            return Expr::prefix(crate::java::syntax::Prefix::Not, other);
                        }
                        _ => (),
                    }
                }
            }
            Expr::binary(*operator, left, right).with_type(Some(&JvmType::Boolean))
        }
        _ => value,
    }
}

pub(super) fn body(body: &mut Body, result: &super::bound::ValueType) {
    for statement in body.iter_mut() {
        children(statement, |nested| self::body(nested, result));
        // Initial binding retains test spelling until choice presentation.
        normalize(statement, result, false);
    }
    overwritten_initializers(body);
    let mut index = 0;
    while index + 1 < body.len() {
        let (before, after) = body.split_at_mut(index + 1);
        let redundant = match (&mut before[index], &after[0]) {
            (Statement::If { condition, yes, no }, Statement::Return(tail)) => {
                no.is_empty()
                    && effects(condition).discardable()
                    && matches!(yes.as_slice(), [Statement::Return(value)] if same_return(value, tail))
            }
            _ => false,
        };
        if redundant {
            body.remove(index);
        } else {
            index += 1;
        }
    }
}

fn same_return(left: &Option<Expr>, right: &Option<Expr>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => same_value(left, right),
        _ => false,
    }
}

/// These identities preserve a value, including legacy raw boolean slots.
/// They do not use Java's declared boolean type as a proof of canonical bits.
pub(super) fn same_value(left: &Expr, right: &Expr) -> bool {
    if let Expression::Group(left) | Expression::Commented(left, _) = &left.kind {
        return same_value(left, right);
    }
    if let Expression::Group(right) | Expression::Commented(right, _) = &right.kind {
        return same_value(left, right);
    }
    if left.facts.ty != right.facts.ty {
        return false;
    }
    // Literal identity survives symbolic binding (for example Enum.FRAME).
    // This is evidence attached to the original literal, never a field read.
    if let (Some(left), Some(right)) = (left.integer_value(), right.integer_value()) {
        return left == right;
    }
    match (&left.kind, &right.kind) {
        (Expression::Atom(left), Expression::Atom(right)) => left == right,
        (Expression::Boolean(left), Expression::Boolean(right)) => left == right,
        (Expression::Null, Expression::Null) => true,
        (Expression::Local { id: left, .. }, Expression::Local { id: right, .. }) => left == right,
        (Expression::Prefix(a, left), Expression::Prefix(b, right)) => {
            std::mem::discriminant(a) == std::mem::discriminant(b) && same_value(left, right)
        }
        (Expression::Binary(a, left), Expression::Binary(b, right)) => {
            std::mem::discriminant(a) == std::mem::discriminant(b)
                && same_value(&left[0], &right[0])
                && same_value(&left[1], &right[1])
        }
        (
            Expression::Cast {
                ty: a, value: left, ..
            },
            Expression::Cast {
                ty: b,
                value: right,
                ..
            },
        ) => a == b && same_value(left, right),
        _ => false,
    }
}

fn preserve_type(value: Expr, ty: Option<JvmType>) -> Expr {
    if value.facts.ty == ty {
        return value;
    }
    let name = match ty {
        Some(JvmType::Int) => "int",
        Some(JvmType::Long) => "long",
        _ => return value,
    };
    Expr::cast(name, value).with_type(ty.as_ref())
}

fn integer(value: &Expr, ty: &JvmType) -> Option<i64> {
    match &value.kind {
        Expression::Group(value) | Expression::Commented(value, _) => integer(value, ty),
        Expression::Integer {
            value, ty: actual, ..
        } if actual == ty => Some(*value),
        _ => None,
    }
}

fn integral_prefix(op: Prefix, value: &Expr, ty: &JvmType) -> Expr {
    if let Expression::Prefix(inner_op, inner) = &value.kind
        && value.facts.ty.as_ref() == Some(ty)
        && matches!(
            (op, inner_op),
            (Prefix::Negate, Prefix::Negate) | (Prefix::Complement, Prefix::Complement)
        )
    {
        return preserve_type(*inner.clone(), Some(ty.clone()));
    }
    // A removed binary literal may have selected long arithmetic. Unary
    // promotion must retain the original width, including before negation.
    let value = if value
        .source_type()
        .and_then(|ty| crate::java::conversions::promoted(ty, ty))
        .as_ref()
        == Some(ty)
    {
        value.clone()
    } else {
        preserve_type(value.clone(), Some(ty.clone()))
    };
    Expr::prefix(op, value).with_type(Some(ty))
}

/// These integral identities remove only a literal, retaining the other
/// operand's evaluation exactly once. A named/mapped constant is intentionally
/// not an integer literal here, even if its numeric value is known.
fn bitwise_identity(expression: &Expr) -> Option<Expr> {
    let ty = expression.facts.ty.as_ref()?;
    if !matches!(ty, JvmType::Int | JvmType::Long) {
        return None;
    }
    let Expression::Binary(operator @ (Operator::And | Operator::Or | Operator::Xor), operands) =
        &expression.kind
    else {
        return None;
    };
    for index in [0, 1] {
        let constant = integer(&operands[index], ty).or_else(|| {
            (*ty == JvmType::Long)
                .then(|| integer(&operands[index], &JvmType::Int))
                .flatten()
        });
        let other = &operands[1 - index];
        match (operator, constant) {
            (Operator::And, Some(-1)) | (Operator::Or | Operator::Xor, Some(0)) => {
                return Some(preserve_type(other.clone(), Some(ty.clone())));
            }
            (Operator::Xor, Some(-1)) => {
                return Some(integral_prefix(Prefix::Complement, other, ty));
            }
            _ => (),
        }
    }
    None
}

fn additive_identity(expression: &Expr) -> Option<Expr> {
    let ty = expression.facts.ty.as_ref()?;
    if !matches!(ty, JvmType::Int | JvmType::Long) {
        return None;
    }
    let Expression::Binary(operator @ (Operator::Add | Operator::Subtract), operands) =
        &expression.kind
    else {
        return None;
    };
    if let Expression::Prefix(Prefix::Negate, right) = &operands[1].kind
        && operands[1].facts.ty.as_ref() == Some(ty)
    {
        let operator = if matches!(operator, Operator::Add) {
            Operator::Subtract
        } else {
            Operator::Add
        };
        return Some(
            Expr::binary(operator, operands[0].clone(), *right.clone()).with_type(Some(ty)),
        );
    }
    if matches!(operator, Operator::Subtract) && integer(&operands[0], ty) == Some(0) {
        return Some(integral_prefix(Prefix::Negate, &operands[1], ty));
    }
    let signed = |operator: &Operator, value: i64| {
        if matches!(operator, Operator::Subtract) {
            value.wrapping_neg()
        } else {
            value
        }
    };
    let wrap = |value: i64| {
        if *ty == JvmType::Int {
            i64::from(value as i32)
        } else {
            value
        }
    };
    if let Some(right) = integer(&operands[1], ty) {
        let right = signed(operator, right);
        if right == 0 {
            return Some(preserve_type(operands[0].clone(), Some(ty.clone())));
        }
        if let Some(left) = integer(&operands[0], ty) {
            let result = wrap(left.wrapping_add(right));
            return Some(Expr::integer(
                result,
                ty.clone(),
                format!("{result}{}", if *ty == JvmType::Long { "L" } else { "" }),
            ));
        }
        if let Expression::Binary(inner @ (Operator::Add | Operator::Subtract), inner_operands) =
            &operands[0].kind
            && operands[0].facts.ty.as_ref() == Some(ty)
            && let Some(previous) = integer(&inner_operands[1], ty)
            && wrap(signed(inner, previous).wrapping_add(right)) == 0
        {
            return Some(preserve_type(inner_operands[0].clone(), Some(ty.clone())));
        }
    }
    if matches!(operator, Operator::Add) && integer(&operands[0], ty) == Some(0) {
        return Some(preserve_type(operands[1].clone(), Some(ty.clone())));
    }
    None
}

/// Removing a default/copy before a certain overwrite uses the same occurrence
/// and abrupt-completion facts. A throwing step is a real boundary: its handler
/// may observe the old binding even when normal execution never reads it.
fn overwritten_initializers(body: &mut Body) {
    let mut index = 0;
    while index < body.len() {
        let local = match &mut body[index] {
            Statement::Simple(Simple::Assign {
                binding,
                value: AssignmentValue::Value(value),
                ..
            }) if binding.synthetic => {
                if !effects(value).discardable() {
                    index += 1;
                    continue;
                }
                binding.local
            }
            _ => {
                index += 1;
                continue;
            }
        };
        let mut replacement = None;
        for (offset, statement) in body[index + 1..].iter_mut().enumerate() {
            let Statement::Simple(value) = statement else {
                break;
            };
            let mut crossed = Effects::default();
            simple(value, &mut crossed);
            if crossed.reads.contains_key(&local) || crossed.throws {
                break;
            }
            if let Simple::Assign { binding, .. } = value
                && binding.local == local
            {
                replacement = Some(index + offset + 1);
                break;
            }
            if crossed.writes.contains_key(&local) {
                break;
            }
        }
        if replacement.is_some() {
            body.remove(index);
        } else {
            index += 1;
        }
    }
}
