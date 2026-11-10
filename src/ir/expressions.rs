//! Source expressions recovered from def/use identity and ordered effects.
//! These folds operate before scheduling, so discarded stores cannot leave stale
//! liveness counts or duplicate a captured array/field read.
use super::lift::{EventKind, RawIr};
use super::*;
use crate::index::{BlockId, IdVec, ValueId};

/// CHECKCAST establishes a reference type; repeating that same check cannot
/// change the value or throw after the first check has succeeded. Recover the
/// identity before scheduling so redundant checks do not become effect barriers
/// or force a multiply-used reference into extra locals.
pub(super) fn recover_casts(raw: &mut RawIr) {
    for id in raw.ir.values.ids() {
        let ValueKind::Cast { value, ty } = &raw.ir.values[id].kind else {
            continue;
        };
        if ty.is_reference() && raw.ir.values[*value].ty == *ty {
            raw.ir.values[id].kind = ValueKind::Alias(*value);
        }
    }
}

pub(super) fn recover_updates(raw: &mut RawIr, class: &crate::classfile::ClassFile) {
    let mut uses: IdVec<ValueId, _> = vec![0usize; raw.ir.values.len()].into();
    for value in &raw.ir.values {
        value.kind.for_each_operand(|operand| uses[operand] += 1);
    }
    for events in &raw.events {
        for event in events {
            if let EventKind::Action(action) = &event.kind {
                for value in action.operands() {
                    uses[value] += 1;
                }
            }
        }
    }
    for block in &raw.ir.blocks {
        for value in block.terminator.operands() {
            uses[value] += 1;
        }
    }

    for block in raw.events.ids() {
        let mut remove = vec![false; raw.events[block].len()];
        for position in 0..raw.events[block].len() {
            let Some((stored, target, target_type)) =
                stored_target(&raw.events[block][position].kind, class)
            else {
                continue;
            };
            if target_type == JvmType::Boolean || target_type.is_reference() {
                continue;
            }
            let mut arithmetic = stored;
            if let ValueKind::Cast { value, ty } = &raw.ir.values[arithmetic].kind {
                if *ty != target_type {
                    continue;
                }
                arithmetic = *value;
            }
            let ValueKind::Binary {
                op: op @ (BinaryOp::Add | BinaryOp::Sub),
                left: read,
                right,
            } = raw.ir.values[arithmetic].kind
            else {
                continue;
            };
            let Some(mut increment) = unit_constant(&raw.ir.values[right].kind) else {
                continue;
            };
            if op == BinaryOp::Sub {
                increment = !increment;
            }
            let matched = reads_target(&target, &raw.ir.values[read].kind);
            if !matched || raw.ir.values[read].block != block {
                continue;
            }
            let Some(read_position) = raw.events[block][..position]
                .iter()
                .position(|event| matches!(event.kind, EventKind::Define(id) if id == read))
            else {
                continue;
            };
            let clear = raw.events[block][read_position + 1..position]
                .iter()
                .enumerate()
                .all(|(offset, event)| {
                    if remove[read_position + 1 + offset] {
                        return true;
                    }
                    match &event.kind {
                        EventKind::Define(value) => !raw.ir.operation_is_observable(*value),
                        EventKind::ThrowPoint => true,
                        EventKind::Allocate(_) | EventKind::ReadTarget(_) => false,
                        EventKind::Action(_) => false,
                    }
                });
            if !clear {
                continue;
            }
            let prefix = uses[read] == 1
                && uses[stored] > 1
                && (arithmetic == stored || uses[arithmetic] == 1);
            let value = if prefix { stored } else { read };
            raw.ir.values[value].kind = ValueKind::Update {
                target,
                increment,
                postfix: !prefix,
            };
            raw.ir.values[value].ty = target_type;
            if prefix {
                raw.ir.values[read].kind = ValueKind::Phi {
                    inputs: vec![],
                    slot: None,
                };
                remove[read_position] = true;
            }
            remove[position] = true;
        }
        let mut index = 0;
        raw.events[block].retain(|_| {
            let keep = !remove[index];
            index += 1;
            keep
        });
    }
    recover_compound_updates(raw, class);
}

/// A JVM dup preserves an lvalue's evaluated receiver/index across its read and
/// write. Give those operands one owner before scheduling counts their uses.
/// The explicit read boundary keeps null/bounds/volatile access before the RHS.
fn recover_compound_updates(raw: &mut RawIr, class: &crate::classfile::ClassFile) {
    use crate::hash::HashSet;
    let live = super::schedule::live_values(raw);
    let mut uses: IdVec<ValueId, _> = vec![0usize; raw.ir.values.len()].into();
    for (id, value) in raw.ir.values.iter_enumerated() {
        if live[id] {
            value.kind.for_each_operand(|operand| uses[operand] += 1);
        }
    }
    for events in &raw.events {
        for event in events {
            if let EventKind::Action(action) = &event.kind {
                for operand in action.operands() {
                    uses[operand] += 1;
                }
            }
        }
    }
    for block in &raw.ir.blocks {
        for operand in block.terminator.operands() {
            uses[operand] += 1;
        }
    }
    for block in raw.events.ids() {
        let mut remove = HashSet::default();
        for position in 0..raw.events[block].len() {
            let Some((stored, target, target_type)) =
                stored_target(&raw.events[block][position].kind, class)
            else {
                continue;
            };
            // Z descriptors are not evidence that arbitrary JVM int producers
            // are canonical booleans. Boolean bit operations retain their IR.
            if !super::type_facts::numeric(&target_type) || uses[stored] != 1 {
                continue;
            }
            let Some(operation) =
                super::update_computation::computation(&raw.ir, stored, &target_type, false)
            else {
                continue;
            };
            let (read, right, op, operation_type) = (
                operation.read,
                operation.right,
                operation.op,
                operation.ty.clone(),
            );
            let same_target = reads_target(&target, &raw.ir.values[read].kind);
            if !same_target || raw.ir.values[read].block != block || uses[read] != 1 {
                continue;
            }
            let Some(start) = raw.events[block][..position]
                .iter()
                .position(|event| matches!(event.kind, EventKind::Define(id) if id == read))
            else {
                continue;
            };
            let start_pc = raw.events[block][start].pc;
            let end_pc = raw.events[block][position].pc;
            if !same_exception_frame(raw, block, start_pc, end_pc, &live) {
                continue;
            }
            let mut spine = HashSet::default();
            let mut value = stored;
            while spine.insert(value) && value != read && uses[value] == 1 {
                value = match raw.ir.values[value].kind {
                    ValueKind::Alias(inner) | ValueKind::Cast { value: inner, .. } => inner,
                    ValueKind::Binary {
                        left,
                        right: operand,
                        ..
                    } if operand == right => left,
                    _ => break,
                };
            }
            if value != read || spine.iter().any(|&value| uses[value] != 1) {
                continue;
            }
            // RHS values born after the target read must remain inside this
            // expression. Shared/external snapshots need their original stores;
            // retaining the primitive update lets scheduling preserve those.
            let mut rhs = HashSet::default();
            let mut pending = vec![(right, 0)];
            let mut confined = true;
            while let Some((value, depth)) = pending.pop() {
                if !rhs.insert(value) {
                    continue;
                }
                let source = &raw.ir.values[value];
                if source.block != block || source.pc <= start_pc {
                    continue;
                }
                if depth >= 32
                    || (uses[value] != 1 && !matches!(source.kind, ValueKind::Literal(_)))
                    // An uninitialized object may span the target load in raw
                    // bytecode. Java cannot begin this RHS allocation before
                    // evaluating its compound lvalue and then resume it after.
                    || matches!(source.kind, ValueKind::New { allocation_pc, .. }
                        if allocation_pc <= start_pc)
                {
                    confined = false;
                    break;
                }
                if !matches!(source.kind, ValueKind::Phi { .. }) {
                    source
                        .kind
                        .for_each_operand(|operand| pending.push((operand, depth + 1)));
                }
            }
            if !confined
                || raw.events[block][start + 1..position]
                    .iter()
                    .any(|event| match event.kind {
                        EventKind::Define(value) => {
                            !spine.contains(&value)
                                && !rhs.contains(&value)
                                && raw.ir.operation_is_observable(value)
                        }
                        EventKind::ReadTarget(value) | EventKind::Allocate(value) => {
                            !rhs.contains(&value)
                        }
                        EventKind::Action(_) => true,
                        EventKind::ThrowPoint => false,
                    })
            {
                continue;
            }
            for index in start + 1..position {
                if matches!(raw.events[block][index].kind, EventKind::Define(value) if spine.contains(&value))
                {
                    remove.insert(index);
                }
            }
            for &value in &spine {
                if value != stored {
                    raw.ir.values[value].kind = ValueKind::Phi {
                        inputs: vec![],
                        slot: None,
                    };
                }
            }
            raw.ir.values[stored].kind = ValueKind::CompoundUpdate {
                target,
                op,
                right,
                operation_type,
            };
            raw.ir.values[stored].ty = target_type;
            raw.ir.values[stored].pc = end_pc;
            raw.events[block][start].kind = EventKind::ReadTarget(stored);
            raw.events[block][position].kind = EventKind::Define(stored);
        }
        let mut index = 0;
        raw.events[block].retain(|_| {
            let keep = !remove.contains(&index);
            index += 1;
            keep
        });
    }
}

fn stored_target(
    event: &EventKind,
    class: &crate::classfile::ClassFile,
) -> Option<(ValueId, UpdateTarget, JvmType)> {
    match event {
        EventKind::Action(Statement::PutField {
            field,
            receiver,
            value,
            ..
        }) => {
            // Reading a final's VM default and publishing once must remain a
            // primitive write for constructor final-initialization projection.
            if field.owner == class.name
                && class.fields.iter().any(|declaration| {
                    declaration.name == field.name
                        && declaration.descriptor == field.descriptor
                        && declaration.access_flags & 0x0010 != 0
                })
            {
                return None;
            }
            Some((
                *value,
                UpdateTarget::Field {
                    field: field.clone(),
                    receiver: *receiver,
                },
                JvmType::parse(&field.descriptor).ok()?,
            ))
        }
        EventKind::Action(Statement::ArrayStore {
            array,
            index,
            value,
            element_type,
        }) => Some((
            *value,
            UpdateTarget::Array {
                array: *array,
                index: *index,
                element_type: element_type.clone(),
            },
            element_type.clone(),
        )),
        _ => None,
    }
}

fn reads_target(target: &UpdateTarget, kind: &ValueKind) -> bool {
    match (target, kind) {
        (
            UpdateTarget::Field { field, receiver },
            ValueKind::Field {
                field: source,
                receiver: object,
            },
        ) => field == source && receiver == object,
        (
            UpdateTarget::Array {
                array,
                index,
                element_type,
            },
            ValueKind::ArrayLoad {
                array: source,
                index: subscript,
                element_type: ty,
            },
        ) => array == source && index == subscript && element_type == ty,
        _ => false,
    }
}

fn unit_constant(kind: &ValueKind) -> Option<bool> {
    match kind {
        ValueKind::Literal(Literal::Int(value)) => match value {
            1 => Some(true),
            -1 => Some(false),
            _ => None,
        },
        ValueKind::Literal(Literal::Long(value)) => match value {
            1 => Some(true),
            -1 => Some(false),
            _ => None,
        },
        ValueKind::Literal(Literal::Float(value)) => {
            if *value == 1.0 {
                Some(true)
            } else if *value == -1.0 {
                Some(false)
            } else {
                None
            }
        }
        ValueKind::Literal(Literal::Double(value)) => {
            if *value == 1.0 {
                Some(true)
            } else if *value == -1.0 {
                Some(false)
            } else {
                None
            }
        }
        _ => None,
    }
}

pub(super) fn recover_array_initializers(
    raw: &mut RawIr,
    reference_assignable: Option<ReferenceAssignable<'_>>,
) {
    let live = super::schedule::live_values(raw);
    for block in raw.events.ids() {
        // Inner arrays complete first, exposing their initializer as one value
        // when a containing multidimensional initializer is considered.
        #[expect(
            clippy::needless_collect,
            reason = "Snapshot array identities before rewriting values and removing events below"
        )]
        let arrays: Vec<_> = raw.events[block]
            .iter()
            .filter_map(|event| match event.kind {
                EventKind::Define(value)
                    if matches!(raw.ir.values[value].kind, ValueKind::NewArray { .. }) =>
                {
                    Some(value)
                }
                _ => None,
            })
            .collect();
        for array in arrays.into_iter().rev() {
            let ValueKind::NewArray {
                element_type,
                dimensions,
            } = &raw.ir.values[array].kind
            else {
                continue;
            };
            if dimensions.len() != 1 {
                continue;
            }
            let ValueKind::Literal(Literal::Int(length)) = raw.ir.values[dimensions[0]].kind else {
                continue;
            };
            if length <= 0 || length > 65535 {
                continue;
            }
            let element_type = element_type.clone();
            let Some(start) = raw.events[block]
                .iter()
                .position(|event| matches!(event.kind, EventKind::Define(value) if value == array))
            else {
                continue;
            };
            let mut written = Vec::new();
            let mut stores = Vec::new();
            for position in start + 1..raw.events[block].len() {
                match &raw.events[block][position].kind {
                    EventKind::Action(Statement::ArrayStore {
                        array: target,
                        index,
                        value,
                        ..
                    }) if *target == array => {
                        let ValueKind::Literal(Literal::Int(index)) = raw.ir.values[*index].kind
                        else {
                            break;
                        };
                        if index < 0
                            || index >= length
                            || *value == array
                            // AASTORE checks the runtime component type. A
                            // Java initializer requiring a downcast would
                            // replace ArrayStoreException by ClassCastException.
                            || (element_type.is_reference()
                                && !matches!(raw.ir.values[*value].kind, ValueKind::Literal(Literal::Null))
                                && super::values::join_type(&raw.ir.values[*value].ty, &element_type) != element_type
                                && !reference_assignable.is_some_and(|assignable| assignable(&raw.ir.values[*value].ty, &element_type)))
                            || written
                                .last()
                                .is_some_and(|&(previous, _)| previous >= index)
                        {
                            break;
                        }
                        written.push((index, *value));
                        stores.push(position);
                        if index == length - 1 {
                            break;
                        }
                    }
                    // The initialized prefix is complete before its first
                    // escape or independent statement. Later writes remain
                    // ordinary stores and retain their original effects.
                    EventKind::Action(_) => break,
                    EventKind::Define(value) => {
                        if raw.ir.values[*value].kind.operands().contains(&array) {
                            break;
                        }
                    }
                    EventKind::ReadTarget(_) => break,
                    EventKind::ThrowPoint | EventKind::Allocate(_) => {}
                }
            }
            if stores.is_empty() {
                continue;
            }
            // In-bounds literal default stores cannot change a fresh array or
            // throw. Leave the allocation at its original position and remove
            // only those stores; signed floating zero must remain distinct.
            if written
                .iter()
                .all(|&(_, value)| array_default(&element_type, &raw.ir.values[value].kind))
            {
                let mut position = 0;
                raw.events[block].retain(|_| {
                    let keep = stores.binary_search(&position).is_err();
                    position += 1;
                    keep
                });
                continue;
            }
            let defaults = length as usize - written.len();
            // Compare the source syntax, excluding unchanged RHS expressions:
            // each indexed store removes six tokens (a [ i ] = ;), while an
            // initializer adds one separator per written value and two tokens
            // per padded default. Its braces replace the size and one comma.
            // Small rows can also remove their temporary and publication into
            // an enclosing literal. Keep those concise tables even when their
            // padding alone costs more than the removed stores.
            if length > 16 && defaults * 2 > written.len() * 5 {
                continue;
            }
            let end = *stores.last().unwrap();
            let allocation_pc = raw.ir.values[array].pc;
            let mut dependencies = crate::hash::HashSet::default();
            let mut work: Vec<_> = written.iter().map(|&(_, value)| value).collect();
            while let Some(value) = work.pop() {
                if !dependencies.insert(value)
                    || matches!(raw.ir.values[value].kind, ValueKind::Phi { .. })
                {
                    continue;
                }
                raw.ir.values[value]
                    .kind
                    .for_each_operand(|operand| work.push(operand));
            }
            if raw.events[block][start + 1..end].iter().any(|event| {
                matches!(event.kind, EventKind::Define(value) if raw.ir.operation_is_observable(value)
                    && !matches!(raw.ir.values[value].kind, ValueKind::Allocation {..}) && !dependencies.contains(&value))
            }) { continue; }
            if !same_exception_frame(raw, block, allocation_pc, raw.events[block][end].pc, &live) {
                continue;
            }
            let elements = if defaults == 0 {
                written.into_iter().map(|(_, value)| value).collect()
            } else {
                let default = raw.ir.values.next_id();
                let literal = match &element_type {
                    JvmType::Boolean
                    | JvmType::Byte
                    | JvmType::Char
                    | JvmType::Short
                    | JvmType::Int => Literal::Int(0),
                    JvmType::Long => Literal::Long(0),
                    JvmType::Float => Literal::Float(0.0),
                    JvmType::Double => Literal::Double(0.0),
                    JvmType::Object(_) | JvmType::Array(_) => Literal::Null,
                    JvmType::Void => continue,
                };
                raw.ir.values.push(Value {
                    local: None,
                    kind: ValueKind::Literal(literal),
                    ty: element_type.clone(),
                    pc: allocation_pc,
                    block,
                });
                let mut elements = vec![default; length as usize];
                for (index, value) in written {
                    elements[index as usize] = value;
                }
                elements
            };
            raw.ir.values[array].pc = raw.events[block][end].pc;
            raw.ir.values[array].kind = ValueKind::ArrayInitializer {
                element_type,
                values: elements,
                allocation_pc,
            };
            raw.events[block][end].kind = EventKind::Define(array);
            raw.events[block][start].kind = EventKind::Allocate(array);
            let mut position = 0;
            raw.events[block].retain(|_| {
                let keep = position == end || stores.binary_search(&position).is_err();
                position += 1;
                keep
            });
        }
    }
}

fn array_default(element_type: &JvmType, value: &ValueKind) -> bool {
    match (element_type, value) {
        (
            JvmType::Boolean | JvmType::Byte | JvmType::Char | JvmType::Short | JvmType::Int,
            ValueKind::Literal(Literal::Int(0)),
        )
        | (JvmType::Long, ValueKind::Literal(Literal::Long(0)))
        | (JvmType::Object(_) | JvmType::Array(_), ValueKind::Literal(Literal::Null)) => true,
        (JvmType::Float, ValueKind::Literal(Literal::Float(value))) => value.to_bits() == 0,
        (JvmType::Double, ValueKind::Literal(Literal::Double(value))) => value.to_bits() == 0,
        _ => false,
    }
}

/// A Java expression has one lexical catch context. Grouping bytecode effects
/// into that expression is sound only when handlers observe the same locals at
/// every possible throw point in the group.
pub(super) fn same_exception_frame(
    raw: &RawIr,
    block: BlockId,
    start: u32,
    end: u32,
    live: &IdVec<ValueId, bool>,
) -> bool {
    if raw.ir.blocks[block].handlers.is_empty() {
        return true;
    }
    for (id, value) in raw.ir.values.iter_enumerated() {
        if !live.get(id).copied().unwrap_or(false) {
            continue;
        }
        let ValueKind::Phi { inputs, .. } = &value.kind else {
            continue;
        };
        let mut source = None;
        for input in inputs {
            if input.block != block || !input.throw_pc.is_some_and(|pc| start <= pc && pc <= end) {
                continue;
            }
            if source.is_some_and(|old| old != input.value) {
                return false;
            }
            source = Some(input.value);
        }
    }
    true
}
