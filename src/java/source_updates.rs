//! Source update forms preserve the scheduled read-before-write operation.
//! A for-step is selected by its exact statement identity, never by searching
//! for an equal-looking update elsewhere in the loop.
use super::{
    conversions::{SourceType, numeric, promoted},
    expr::{ExprPlanner, binary_operator},
    syntax::Expr,
};
use crate::ir::update_computation::{Computation, computation, inline_alias};
use crate::{
    Result,
    classfile::JvmType,
    ir::{
        BinaryOp, BlockId, Literal, LocalId, Member, MethodIr, Statement, ValueId, ValueKind,
        ValueUse,
    },
    structure::Stmt,
};

pub(super) fn field(
    expr: &ExprPlanner<'_, '_>,
    field: &Member,
    receiver: Option<ValueId>,
    value: ValueId,
) -> Result<Option<Expr>> {
    if field.descriptor.len() != 1 {
        return Ok(None);
    }
    if expr
        .cx
        .program
        .resolve_field(&field.owner, &field.name, &field.descriptor)
        .is_some_and(|member| expr.cx.program.field(member).access_flags & 0x0010 != 0)
    {
        return Ok(None);
    }
    let ty = JvmType::parse(&field.descriptor)?;
    let Some(operation) = computation(expr.ir, value, &ty, false) else {
        return Ok(None);
    };
    if expr.ir.values[operation.read].local.is_some() {
        return Ok(None);
    }
    let ValueKind::Field {
        field: read_field,
        receiver: read_receiver,
    } = &expr.ir.values[operation.read].kind
    else {
        return Ok(None);
    };
    let same_field = field == read_field
        || expr
            .cx
            .program
            .resolve_field(&field.owner, &field.name, &field.descriptor)
            .zip(expr.cx.program.resolve_field(
                &read_field.owner,
                &read_field.name,
                &read_field.descriptor,
            ))
            .is_some_and(|(write, read)| write == read);
    let same_receiver = match (receiver, *read_receiver) {
        (None, None) => true,
        (Some(write), Some(read)) => same_operand(expr.ir, write, read),
        _ => false,
    };
    if !same_field || !same_receiver {
        return Ok(None);
    }
    plan_update(
        expr,
        expr.field_assignment(field, receiver)?,
        &ty,
        operation,
    )
}

pub(super) fn array(
    expr: &ExprPlanner<'_, '_>,
    array: ValueId,
    index: ValueId,
    value: ValueId,
    ty: &JvmType,
) -> Result<Option<Expr>> {
    let Some(operation) = computation(expr.ir, value, ty, false) else {
        return Ok(None);
    };
    if expr.ir.values[operation.read].local.is_some() {
        return Ok(None);
    }
    let ValueKind::ArrayLoad {
        array: read_array,
        index: read_index,
        element_type,
    } = &expr.ir.values[operation.read].kind
    else {
        return Ok(None);
    };
    if element_type != ty
        || !same_operand(expr.ir, array, *read_array)
        || !same_operand(expr.ir, index, *read_index)
    {
        return Ok(None);
    }
    let place = expr
        .array_receiver(array, ty)?
        .index(expr.expected(index, &JvmType::Int)?);
    plan_update(expr, place, ty, operation)
}

fn same_operand(ir: &MethodIr, write: ValueId, read: ValueId) -> bool {
    if write == read {
        return ir.values[write].local.is_some()
            || matches!(
                ir.values[write].kind,
                ValueKind::Parameter { .. } | ValueKind::Literal(_)
            );
    }
    // Literal indexes/null receivers carry values rather than evaluated object
    // identities; separate bytecode constant loads can denote the same operand.
    if ir.values[write].local.is_none() && ir.values[read].local.is_none() {
        match (&ir.values[write].kind, &ir.values[read].kind) {
            (ValueKind::Literal(Literal::Int(a)), ValueKind::Literal(Literal::Int(b))) => a == b,
            (ValueKind::Literal(Literal::Null), ValueKind::Literal(Literal::Null)) => true,
            (ValueKind::Alias(value), _) => same_operand(ir, *value, read),
            (_, ValueKind::Alias(value)) => same_operand(ir, write, *value),
            _ => false,
        }
    } else {
        false
    }
}

fn plan_update(
    expr: &ExprPlanner<'_, '_>,
    place: Expr,
    target: &JvmType,
    operation: Computation<'_>,
) -> Result<Option<Expr>> {
    let Computation { op, right, ty, .. } = operation;
    if matches!(op, BinaryOp::Add | BinaryOp::Sub)
        && promoted(target, target).as_ref() == Some(ty)
        && expr.ir.values[right].local.is_none()
        && let Some(direction) = unit(&expr.ir.values[right].kind)
    {
        let increment = (direction == 1) == (op == BinaryOp::Add);
        return Ok(Some(Expr::update(place, increment, true)));
    }
    Ok(Some(plan_operation(expr, place, target, op, right, ty)?))
}

pub(super) fn plan_operation(
    expr: &ExprPlanner<'_, '_>,
    place: Expr,
    target: &JvmType,
    op: BinaryOp,
    right: ValueId,
    ty: &JvmType,
) -> Result<Expr> {
    let shift = matches!(op, BinaryOp::Shl | BinaryOp::Shr | BinaryOp::Ushr);
    let expected = if shift { &JvmType::Int } else { ty };
    let (op, mut right_syntax, actual) =
        match super::literals::additive_operand(expr.ir, op, right, ty) {
            Some((op, literal)) => (
                op,
                super::expr::literal_expression(&literal),
                SourceType::value(&expr.ir.values[right].ty),
            ),
            None => {
                let (expression, actual) = expr.typed_value(right, expected)?;
                (op, expression, actual)
            }
        };
    if !shift
        && *target != JvmType::Boolean
        && actual
            .types()
            .any(|actual| promoted(target, actual).as_ref() != Some(ty))
    {
        // The old read may have been explicitly widened before arithmetic.
        // Make the RHS select that same operation width, then let compound
        // assignment perform the original final narrowing to the lvalue type.
        right_syntax = Expr::cast(expr.cx.types.ty(ty), right_syntax.grouped());
    }
    Ok(Expr::assignment(
        place,
        right_syntax,
        Some(binary_operator(op)),
    ))
}

fn unit(kind: &ValueKind) -> Option<i32> {
    match kind {
        ValueKind::Literal(Literal::Int(1) | Literal::Long(1)) => Some(1),
        ValueKind::Literal(Literal::Int(-1) | Literal::Long(-1)) => Some(-1),
        ValueKind::Literal(Literal::Float(value)) if *value == 1.0 || *value == -1.0 => {
            Some(*value as i32)
        }
        ValueKind::Literal(Literal::Double(value)) if *value == 1.0 || *value == -1.0 => {
            Some(*value as i32)
        }
        _ => None,
    }
}

pub(super) fn assignment(
    expr: &ExprPlanner<'_, '_>,
    local: LocalId,
    value: ValueUse,
) -> Result<Option<Expr>> {
    let Some(operation) = local_computation(expr.ir, local, value) else {
        return Ok(None);
    };
    if expr.ir.locals[local].ty == JvmType::Boolean
        && let Some(right) = inline_alias(expr.ir, operation.right)
        && let ValueKind::Literal(Literal::Int(constant @ (0 | 1))) = expr.ir.values[right].kind
        && expr
            .boolean_identity(operation.op, operation.read, constant != 0)
            .is_some()
    {
        // Let the ordinary Boolean binder remove identities or spell a toggle
        // as !value; compound syntax is useful for the remaining eager work.
        return Ok(None);
    }
    plan_update(
        expr,
        Expr::local(local, expr.local_name(local), &expr.ir.locals[local].ty),
        &expr.ir.locals[local].ty,
        operation,
    )
}

fn local_computation(ir: &MethodIr, local: LocalId, usage: ValueUse) -> Option<Computation<'_>> {
    let value = usage.value();
    let ty = &ir.locals[local].ty;
    if (!numeric(ty) && *ty != JvmType::Boolean)
        || usage.local(ir).is_some()
        || &ir.values[value].ty != ty
    {
        return None;
    }
    let operation = computation(ir, value, ty, true)?;
    (ir.values[operation.read].local == Some(local)).then_some(operation)
}

pub(super) fn integer_step(ir: &MethodIr, local: LocalId, value: ValueUse) -> Option<i32> {
    if ir.locals[local].ty != JvmType::Int {
        return None;
    }
    let operation = local_computation(ir, local, value)?;
    let right = &ir.values[inline_alias(ir, operation.right)?];
    let ValueKind::Literal(Literal::Int(amount)) = right.kind else {
        return None;
    };
    match operation.op {
        BinaryOp::Add => Some(amount),
        BinaryOp::Sub => Some(amount.wrapping_neg()),
        _ => None,
    }
}

pub(super) struct ForStep {
    pub local: LocalId,
    pub block: BlockId,
    pub statement: usize,
}

pub(super) fn loop_step(
    expr: &ExprPlanner<'_, '_>,
    body: &[Stmt],
    header: BlockId,
    guard_end: usize,
    continuation: Option<BlockId>,
) -> Result<Option<ForStep>> {
    if let Some(block) = continuation {
        let [Statement::Assign { local, .. }] = expr.ir.blocks[block].statements.as_slice() else {
            anyhow::bail!("loop continuation block {block} is not one local assignment");
        };
        // The structurer owns this iteration boundary. Its continue edges must
        // execute the update, unlike heuristic movement of a trailing statement.
        return Ok(Some(ForStep {
            local: *local,
            block,
            statement: 0,
        }));
    }
    if guard_end == 0 {
        return Ok(None);
    }
    let Some(Stmt::Basic(block)) = body.last() else {
        return Ok(None);
    };
    let Some((statement, Statement::Assign { local, value })) = expr.ir.blocks[*block]
        .statements
        .iter()
        .enumerate()
        .next_back()
    else {
        return Ok(None);
    };
    let Some(_) = local_computation(expr.ir, *local, *value) else {
        return Ok(None);
    };
    let guard_reads_local = body[..guard_end].iter().any(|statement| {
        let Stmt::If { condition, .. } = statement else {
            return false;
        };
        condition.heads().iter().any(|head| {
            expr.ir.blocks[*head]
                .terminator
                .operands()
                .iter()
                .any(|value| {
                    super::local_flow::value_reads(expr.ir, ValueUse::Reference(*value))
                        .contains(local)
                })
        })
    });
    if !guard_reads_local {
        return Ok(None);
    }
    let mut pending = vec![body];
    let mut occurrences = 0;
    while let Some(body) = pending.pop() {
        for statement in body {
            match statement {
                Stmt::Basic(id) | Stmt::Statements { block: id, .. } => {
                    if id == block {
                        occurrences += 1;
                    }
                }
                Stmt::Continue { header: target } if *target == header => return Ok(None),
                Stmt::Dispatch { .. } | Stmt::BoundRegion { .. } | Stmt::Jump(_) => {
                    return Ok(None);
                }
                Stmt::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    pending.push(then_body);
                    pending.push(else_body);
                }
                Stmt::Loop {
                    body, continuation, ..
                } => {
                    pending.push(body);
                    if let Some(id) = continuation
                        && id == block
                    {
                        occurrences += 1;
                    }
                }
                Stmt::Label { body, .. } | Stmt::Synchronized { body, .. } => pending.push(body),
                Stmt::Switch { cases, .. } => {
                    pending.extend(cases.iter().map(|case| case.body.as_slice()))
                }
                Stmt::Try { body, catches } => {
                    pending.push(body);
                    pending.extend(catches.iter().map(|catch| catch.body.as_slice()));
                }
                Stmt::TryFinally { body, cleanup, .. } => {
                    pending.push(body);
                    pending.push(cleanup);
                }
                _ => (),
            }
        }
    }
    Ok((occurrences == 1).then_some(ForStep {
        local: *local,
        block: *block,
        statement,
    }))
}
