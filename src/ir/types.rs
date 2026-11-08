//! Source type facts flow through SSA dependencies, independently of block visit
//! order. In particular an array load cannot keep its provisional entry-frame
//! type after the array's phi has converged.
use super::lift::{EventKind, RawIr};
use super::*;
use crate::classfile::MethodDescriptor;
use crate::index::IdVec;
use std::collections::VecDeque;

pub(super) fn refine(raw: &mut RawIr, reference_assignable: Option<ReferenceAssignable<'_>>) {
    let count = raw.ir.values.len();
    let mut users: IdVec<ValueId, _> = vec![vec![]; count].into();
    for (id, value) in raw.ir.values.iter_enumerated() {
        value
            .kind
            .for_each_operand(|operand| users[operand].push(id));
    }
    // A provisional entry-frame type is not evidence. In particular a cycle of
    // phis seeded by null must not manufacture an Object definition and prevent
    // the real byte[] (or other reference) definitions from flowing through it.
    let mut facts: IdVec<ValueId, Option<JvmType>> = raw
        .ir
        .values
        .iter()
        .map(|value| match &value.kind {
            ValueKind::Phi { .. } | ValueKind::Alias(_) | ValueKind::Literal(Literal::Null) => None,
            ValueKind::ArrayLoad { element_type, .. }
                if element_type.is_reference()
                    || matches!(element_type, JvmType::Byte | JvmType::Boolean) =>
            {
                None
            }
            _ => Some(value.ty.clone()),
        })
        .collect();
    let mut queue: VecDeque<_> = raw.ir.values.ids().collect();
    let mut queued: IdVec<ValueId, _> = vec![true; count].into();
    while let Some(id) = queue.pop_front() {
        queued[id] = false;
        let inferred = match &raw.ir.values[id].kind {
            ValueKind::Phi { inputs, .. } => {
                let mut result: Option<JvmType> = None;
                for input in inputs {
                    if input.value == id {
                        continue;
                    }
                    let Some(ty) = &facts[input.value] else {
                        continue;
                    };
                    result = Some(if let Some(old) = result {
                        join(&old, ty, reference_assignable)
                    } else {
                        ty.clone()
                    });
                }
                result
            }
            ValueKind::Alias(source) => facts[*source].clone(),
            ValueKind::ArrayLoad {
                array,
                element_type,
                ..
            } if element_type.is_reference()
                || matches!(element_type, JvmType::Byte | JvmType::Boolean) =>
            {
                facts[*array]
                    .as_ref()
                    .map(|array| array_element(element_type, array))
            }
            _ => facts[id].clone(),
        };
        // The conservative fallback is not a least upper bound for unknown
        // sibling types. Once widened to Object, a cyclic phi must not narrow
        // back to an interface merely because another incoming fact changed.
        // Joining the previous fact makes every update a monotone widening.
        let inferred = match (&facts[id], inferred) {
            (Some(old), Some(new)) => Some(join(old, &new, reference_assignable)),
            (Some(old), None) => Some(old.clone()),
            (None, new) => new,
        };
        if inferred != facts[id] {
            facts[id] = inferred;
            for &user in &users[id] {
                if !queued[user] {
                    queued[user] = true;
                    queue.push_back(user);
                }
            }
        }
    }
    for (value, fact) in raw.ir.values.iter_mut().zip(facts) {
        if let Some(ty) = fact {
            if let ValueKind::ArrayLoad { element_type, .. } = &mut value.kind {
                *element_type = ty.clone();
            }
            value.ty = ty;
        }
    }
    for events in &mut raw.events {
        for event in events {
            if let EventKind::Action(Statement::ArrayStore {
                array,
                element_type,
                ..
            }) = &mut event.kind
            {
                *element_type = array_element(element_type, &raw.ir.values[*array].ty);
            }
        }
    }
    refine_booleans(raw, &users);
}

fn join(
    left: &JvmType,
    right: &JvmType,
    reference_assignable: Option<ReferenceAssignable<'_>>,
) -> JvmType {
    // Keep a proven supertype supplied by an incoming definition. Unknown
    // hierarchy facts retain the conservative frame join; null remains bottom.
    if left != right
        && left.is_reference()
        && right.is_reference()
        && let Some(assignable) = reference_assignable
    {
        if assignable(left, right) {
            return right.clone();
        }
        if assignable(right, left) {
            return left.clone();
        }
    }
    super::values::join_type(left, right)
}

fn array_element(opcode_element: &JvmType, array: &JvmType) -> JvmType {
    let JvmType::Array(component) = array else {
        return opcode_element.clone();
    };
    if opcode_element.is_reference() && component.is_reference() {
        return *component.clone();
    }
    if matches!(opcode_element, JvmType::Byte | JvmType::Boolean)
        && matches!(**component, JvmType::Byte | JvmType::Boolean)
    {
        return *component.clone();
    }
    opcode_element.clone()
}

fn refine_booleans(raw: &mut RawIr, users: &IdVec<ValueId, Vec<ValueId>>) {
    let count = raw.ir.values.len();
    // A boolean-capable phi cycle is legal only if every external definition is
    // boolean-capable. Start with the maximal solution, then remove bad nodes.
    let mut possible: IdVec<ValueId, _> = raw
        .ir
        .values
        .iter()
        .map(|value| {
            value.ty == JvmType::Boolean
                || matches!(
                    value.kind,
                    ValueKind::Literal(Literal::Int(0 | 1))
                        | ValueKind::Phi { .. }
                        | ValueKind::Alias(_)
                        | ValueKind::Binary {
                            op: BinaryOp::And | BinaryOp::Or | BinaryOp::Xor,
                            ..
                        }
                )
        })
        .collect();
    let mut queue: VecDeque<_> = raw.ir.values.ids().collect();
    while let Some(id) = queue.pop_front() {
        if !possible[id] {
            continue;
        }
        let capable = match &raw.ir.values[id].kind {
            ValueKind::Phi { inputs, .. } => {
                !inputs.is_empty() && inputs.iter().all(|input| possible[input.value])
            }
            ValueKind::Alias(source) => possible[*source],
            ValueKind::Binary {
                op: BinaryOp::And | BinaryOp::Or | BinaryOp::Xor,
                left,
                right,
            } => possible[*left] && possible[*right],
            _ => true,
        };
        if !capable {
            possible[id] = false;
            queue.extend(users[id].iter().copied());
        }
    }
    let mut demands = vec![];
    for (id, value) in raw.ir.values.iter_enumerated() {
        if value.ty == JvmType::Boolean {
            demands.push(id);
        }
        match &value.kind {
            ValueKind::Call { args, .. } | ValueKind::New { args, .. } => {
                let descriptor = match &value.kind {
                    ValueKind::Call { method, .. } => &method.descriptor,
                    ValueKind::New { descriptor, .. } => descriptor,
                    _ => unreachable!(),
                };
                if let Ok(signature) = MethodDescriptor::parse(descriptor) {
                    for (&argument, ty) in args.iter().zip(signature.parameters) {
                        if ty == JvmType::Boolean {
                            demands.push(argument);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    for block in &raw.ir.blocks {
        if raw.ir.return_type == JvmType::Boolean
            && let Terminator::Return(Some(value)) = block.terminator
        {
            demands.push(value);
        }
    }
    // A local used only in logical comparisons has no descriptor to demand
    // boolean. Infer that role over its complete producer/use component. Raw
    // slots survive Boolean storage, eager operators and Boolean equality.
    let truth_only = truth_only_values(raw, &possible);
    for block in &raw.ir.blocks {
        if let Terminator::Branch { condition, .. } = &block.terminator
            && boolean_comparison(condition, &possible)
            && [condition.left, condition.right]
                .into_iter()
                .all(|value| boolean_literal(raw, value) || truth_only[value])
        {
            demands.extend([condition.left, condition.right]);
        }
    }
    for events in &raw.events {
        for event in events {
            match &event.kind {
                EventKind::Action(Statement::PutField { field, value, .. })
                    if field.descriptor == "Z" =>
                {
                    demands.push(*value)
                }
                EventKind::Action(Statement::ArrayStore {
                    element_type: JvmType::Boolean,
                    value,
                    ..
                }) => demands.push(*value),
                EventKind::Action(Statement::ConstructorCall { method, args, .. }) => {
                    if let Ok(signature) = MethodDescriptor::parse(&method.descriptor) {
                        for (&argument, ty) in args.iter().zip(signature.parameters) {
                            if ty == JvmType::Boolean {
                                demands.push(argument);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut seen: IdVec<ValueId, _> = vec![false; count].into();
    while let Some(id) = demands.pop() {
        if seen[id] || !possible[id] {
            continue;
        }
        seen[id] = true;
        match &raw.ir.values[id].kind {
            ValueKind::Phi { .. }
            | ValueKind::Alias(_)
            | ValueKind::Binary {
                op: BinaryOp::And | BinaryOp::Or | BinaryOp::Xor,
                ..
            } => {
                raw.ir.values[id]
                    .kind
                    .for_each_operand(|operand| demands.push(operand));
                raw.ir.values[id].ty = JvmType::Boolean;
            }
            _ => {}
        }
    }
}

fn boolean_comparison(condition: &Condition, possible: &IdVec<ValueId, bool>) -> bool {
    matches!(condition.op, CompareOp::Eq | CompareOp::Ne)
        && possible[condition.left]
        && possible[condition.right]
}

fn boolean_literal(raw: &RawIr, value: ValueId) -> bool {
    matches!(
        raw.ir.values[value].kind,
        ValueKind::Literal(Literal::Int(0 | 1))
    )
}

fn boolean_flow(kind: &ValueKind) -> bool {
    matches!(
        kind,
        ValueKind::Phi { .. }
            | ValueKind::Alias(_)
            | ValueKind::Binary {
                op: BinaryOp::And | BinaryOp::Or | BinaryOp::Xor,
                ..
            }
    )
}

/// A descriptor-free predicate component needs both compatible producers and
/// exclusively logical consumers. Copies, phi cycles, eager operators and paired
/// equalities connect that role in both directions: a numeric use of either
/// comparand or any copied value prevents partially retyping the component.
/// Literal 0/1 operands remain flexible at each occurrence and never connect
/// otherwise independent locals through a shared constant.
fn truth_only_values(raw: &RawIr, possible: &IdVec<ValueId, bool>) -> IdVec<ValueId, bool> {
    let mut truth_only = possible.clone();
    let mut peers: IdVec<ValueId, Vec<ValueId>> = vec![vec![]; raw.ir.values.len()].into();
    let mut connect = |left, right| {
        if !boolean_literal(raw, left) && !boolean_literal(raw, right) {
            peers[left].push(right);
            peers[right].push(left);
        }
    };
    for (id, value) in raw.ir.values.iter_enumerated() {
        match &value.kind {
            kind if boolean_flow(kind) => {
                kind.for_each_operand(|operand| connect(id, operand));
            }
            ValueKind::Call {
                method,
                receiver,
                args,
                ..
            } => {
                if let Some(receiver) = receiver {
                    truth_only[*receiver] = false;
                }
                numeric_arguments(&method.descriptor, args, &mut truth_only);
            }
            ValueKind::New {
                descriptor, args, ..
            } => numeric_arguments(descriptor, args, &mut truth_only),
            kind => kind.for_each_operand(|operand| truth_only[operand] = false),
        }
    }
    for events in &raw.events {
        for event in events {
            if let EventKind::Action(action) = &event.kind {
                match action {
                    Statement::PutField {
                        field,
                        receiver,
                        value,
                        ..
                    } => {
                        if let Some(receiver) = receiver {
                            truth_only[*receiver] = false;
                        }
                        if field.descriptor != "Z" {
                            truth_only[*value] = false;
                        }
                    }
                    Statement::ArrayStore {
                        array,
                        index,
                        value,
                        element_type,
                    } => {
                        truth_only[*array] = false;
                        truth_only[*index] = false;
                        if *element_type != JvmType::Boolean {
                            truth_only[*value] = false;
                        }
                    }
                    Statement::ConstructorCall {
                        receiver,
                        method,
                        args,
                        ..
                    } => {
                        truth_only[*receiver] = false;
                        numeric_arguments(&method.descriptor, args, &mut truth_only);
                    }
                    _ => {
                        for operand in action.operands() {
                            truth_only[operand] = false;
                        }
                    }
                }
            }
        }
    }
    for block in &raw.ir.blocks {
        match &block.terminator {
            Terminator::Branch { condition, .. } if boolean_comparison(condition, possible) => {
                connect(condition.left, condition.right);
            }
            Terminator::Return(_) if raw.ir.return_type == JvmType::Boolean => {}
            terminator => {
                for operand in terminator.operands() {
                    truth_only[operand] = false;
                }
            }
        }
    }
    let mut pending: Vec<_> = truth_only
        .iter_enumerated()
        .filter_map(|(id, &truth)| (!truth).then_some(id))
        .collect();
    while let Some(value) = pending.pop() {
        for &peer in &peers[value] {
            if truth_only[peer] {
                truth_only[peer] = false;
                pending.push(peer);
            }
        }
    }
    truth_only
}

fn numeric_arguments(descriptor: &str, args: &[ValueId], truth_only: &mut IdVec<ValueId, bool>) {
    let signature = MethodDescriptor::parse(descriptor).ok();
    for (index, &argument) in args.iter().enumerate() {
        if signature
            .as_ref()
            .and_then(|signature| signature.parameters.get(index))
            != Some(&JvmType::Boolean)
        {
            truth_only[argument] = false;
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/boolean_types.rs"]
mod tests;
