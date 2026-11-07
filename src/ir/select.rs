//! Recover the source owner of a choice. One result becomes a lazy expression;
//! operands of a shared consumer belong to the complete operation in each arm.
//! Scheduling retains each arm's original observable evaluation order.
use super::evaluation;
use super::lift::{Event, EventKind, RawIr};
use super::local_identity::incoming_local;
use super::*;
use crate::index::{BlockId, IdVec, ValueId};
use std::collections::HashSet;

pub(super) fn recover(raw: &mut RawIr) {
    loop {
        let live = super::schedule::live_values(raw);
        let mut predecessors: IdVec<BlockId, _> = vec![Vec::new(); raw.ir.blocks.len()].into();
        let mut exceptional_entry: IdVec<BlockId, _> = vec![false; raw.ir.blocks.len()].into();
        for (source, block) in raw.ir.blocks.iter_enumerated() {
            for target in block.terminator.successors() {
                predecessors[target].push(source);
            }
            for handler in crate::cfg::dispatch::effective(&block.handlers) {
                exceptional_entry[handler.target] = true;
            }
        }
        // A short-circuit predicate can share one result arm. Recover its
        // conditional evaluation before the result diamond, so values already
        // on the stack need not acquire storage merely to cross the tests.
        if recover_short_circuit(raw, &predecessors, &exceptional_entry, &live) {
            continue;
        }
        let mut changed = false;
        for header in raw.ir.blocks.ids() {
            let Terminator::Branch {
                condition,
                then_target,
                else_target,
            } = &raw.ir.blocks[header].terminator
            else {
                continue;
            };
            let (condition, left, right) = (condition.clone(), *then_target, *else_target);
            if left == right || left == header || right == header {
                continue;
            }
            let (Terminator::Goto(join), Terminator::Goto(other)) = (
                &raw.ir.blocks[left].terminator,
                &raw.ir.blocks[right].terminator,
            ) else {
                continue;
            };
            let join = *join;
            if join != *other || [header, left, right].contains(&join) {
                continue;
            }
            if predecessors[left] != [header]
                || predecessors[right] != [header]
                || predecessors[join].len() != 2
                || !predecessors[join].contains(&left)
                || !predecessors[join].contains(&right)
            {
                continue;
            }
            if [left, right, join]
                .iter()
                .any(|&block| exceptional_entry[block])
            {
                continue;
            }
            // Pure control/return blocks can sit just outside an exception
            // table range. A source expression needs the same handlers only
            // at operations that can actually transfer to them.
            let throwing: Vec<_> = [header, left, right, join]
                .into_iter()
                .filter(|&block| {
                    matches!(raw.ir.blocks[block].terminator, Terminator::Throw { .. })
                        || raw.events[block].iter().any(|event| match event.kind {
                            EventKind::Define(value) => raw.ir.values[value].may_throw(),
                            EventKind::Action(_)
                            | EventKind::Allocate(_)
                            | EventKind::ReadTarget(_) => true,
                            EventKind::ThrowPoint => false,
                        })
                })
                .collect();
            let scope = throwing.first().copied().unwrap_or(header);
            if throwing
                .iter()
                .any(|&block| raw.ir.blocks[block].handlers != raw.ir.blocks[scope].handlers)
            {
                continue;
            }
            let handlers = raw.ir.blocks[scope].handlers.clone();
            if recover_expression(
                raw,
                [header, left, right, join],
                condition,
                &handlers,
                &live,
            ) || project_consumer(raw, [left, right], join, &handlers, &live)
            {
                changed = true;
                break;
            }
        }
        if !changed {
            break;
        }
    }
}

fn recover_expression(
    raw: &mut RawIr,
    [header, left, right, join]: [BlockId; 4],
    condition: Condition,
    handlers: &[crate::cfg::HandlerEdge],
    live: &IdVec<ValueId, bool>,
) -> bool {
    let phis: Vec<_> = raw
        .ir
        .values
        .iter_enumerated()
        .filter_map(|(id, value)| {
            (live[id] && value.block == join && matches!(value.kind, ValueKind::Phi { .. }))
                .then_some(id)
        })
        .collect();
    if phis.len() != 1 {
        return false;
    }
    // Catch locals are also outputs. Stable incoming snapshots can be
    // taken before the choice, but an arm-dependent local needs the CFG.
    if !stable_exception_inputs(raw, &[left, right], live) {
        return false;
    }
    let value = phis[0];
    let ValueKind::Phi { inputs, slot } = &raw.ir.values[value].kind else {
        unreachable!()
    };
    if inputs.len() != 2 || inputs.iter().any(|input| input.throw_pc.is_some()) {
        return false;
    }
    // A local retaining its incoming version on one edge is a guarded
    // update of existing storage. Keep that statement ownership (and
    // loop continuations), rather than manufacture x = test ? y : x.
    if slot.is_some_and(|slot| {
        inputs.iter().any(|input| {
            ![left, right].contains(&raw.ir.values[input.value].block)
                && incoming_local(raw, header, input.value, slot)
        })
    }) {
        return false;
    }
    let Some(then_value) = inputs
        .iter()
        .find(|input| input.block == left)
        .map(|input| input.value)
    else {
        return false;
    };
    let Some(else_value) = inputs
        .iter()
        .find(|input| input.block == right)
        .map(|input| input.value)
    else {
        return false;
    };
    if !region_is_expression(raw, left, &[then_value], Some(value), live)
        || !region_is_expression(raw, right, &[else_value], Some(value), live)
    {
        return false;
    }
    let pc = raw.ir.blocks[left].pc.max(raw.ir.blocks[right].pc);
    raw.ir.values[value].kind = ValueKind::Select {
        condition,
        then_value,
        else_value,
    };
    raw.ir.values[value].pc = pc;
    raw.ir.values[value].block = header;
    raw.ir.blocks[header].handlers = handlers.to_vec();
    let mut regions = [Vec::new(), Vec::new()];
    for (index, arm) in [left, right].into_iter().enumerate() {
        let events = std::mem::take(&mut raw.events[arm]);
        for event in &events {
            if let EventKind::Define(id) = event.kind {
                raw.ir.values[id].block = header;
                regions[index].push(id);
            }
        }
        raw.events[header].extend(events);
        raw.ir.blocks[arm].terminator = Terminator::Unreachable;
        raw.ir.blocks[arm].handlers.clear();
    }
    raw.conditional_regions.insert(value, regions);
    raw.events[header].push(Event {
        pc,
        kind: EventKind::Define(value),
    });
    let events = std::mem::take(&mut raw.events[join]);
    raw.events[header].extend(events);
    raw.ir.blocks[header].terminator =
        std::mem::replace(&mut raw.ir.blocks[join].terminator, Terminator::Unreachable);
    raw.ir.blocks[join].handlers.clear();
    for source in &mut raw.ir.values {
        if source.block == join {
            source.block = header;
        }
        if let ValueKind::Phi { inputs, .. } = &mut source.kind {
            for input in inputs {
                if input.block == join
                    || input.throw_pc.is_some() && [left, right].contains(&input.block)
                {
                    input.block = header;
                }
            }
        }
    }
    true
}

/// Bytecode can share an operation after choosing all its operands. Rebind that
/// first consumer to each incoming frame before scheduling would give its stack
/// tuple synthetic storage. A value consumer leaves one result phi; an action
/// leaves none. Existing expression ownership and ordering then apply normally.
fn project_consumer(
    raw: &mut RawIr,
    arms: [BlockId; 2],
    join: BlockId,
    handlers: &[crate::cfg::HandlerEdge],
    live: &IdVec<ValueId, bool>,
) -> bool {
    let Some(index) = raw.events[join]
        .iter()
        .position(|event| !matches!(event.kind, EventKind::ThrowPoint))
    else {
        return false;
    };
    let event = raw.events[join][index].clone();
    let (result, operands) = match &event.kind {
        // Constructors and monitors establish lifetimes beyond one operation.
        EventKind::Action(Statement::ConstructorCall { .. } | Statement::Monitor { .. }) => {
            return false;
        }
        EventKind::Action(action) => (None, action.operands()),
        EventKind::Define(value)
            if !matches!(
                raw.ir.values[*value].kind,
                ValueKind::Allocation { .. }
                    | ValueKind::New { .. }
                    | ValueKind::ArrayInitializer { .. }
                    | ValueKind::Select { .. }
            ) =>
        {
            (Some(*value), raw.ir.values[*value].kind.operands())
        }
        _ => return false,
    };
    let selected: HashSet<_> = operands
        .iter()
        .copied()
        .filter(|&value| {
            raw.ir.values[value].block == join
                && matches!(raw.ir.values[value].kind, ValueKind::Phi { slot: None, .. })
        })
        .collect();
    if selected.is_empty() {
        return false;
    }
    let mut incoming = std::collections::HashMap::new();
    for (value, source) in raw.ir.values.iter_enumerated() {
        if !live[value] || source.block != join {
            continue;
        }
        let ValueKind::Phi { inputs, .. } = &source.kind else {
            if operands.contains(&value) {
                return false;
            }
            continue;
        };
        if inputs.len() != 2 || inputs.iter().any(|input| input.throw_pc.is_some()) {
            return false;
        }
        let values: Option<Vec<_>> = arms
            .iter()
            .map(|&arm| {
                inputs
                    .iter()
                    .find(|input| input.block == arm)
                    .map(|input| input.value)
            })
            .collect();
        let Some(values) = values else { return false };
        incoming.insert(value, values);
    }
    // An operand tuple is owned here only if no later computation, local state,
    // or exceptional snapshot also needs the selected stack values.
    let reads_selected =
        |operands: Vec<ValueId>| operands.iter().any(|value| selected.contains(value));
    if raw.ir.values.iter_enumerated().any(|(id, value)| {
        live[id] && Some(id) != result && reads_selected(value.kind.operands())
    }) || raw.events.iter_enumerated().any(|(block, events)| {
        events.iter().enumerate().any(|(position, event)| {
            (block != join || position != index)
                && matches!(&event.kind, EventKind::Action(action) if reads_selected(action.operands()))
        })
    }) || raw.ir.blocks.iter().any(|block| reads_selected(block.terminator.operands())) {
        return false;
    }
    let prefix: Vec<_> = raw.events[join].drain(..=index).collect();
    let mut results = Vec::new();
    for (side, arm) in arms.into_iter().enumerate() {
        let bind = |value| incoming.get(&value).map_or(value, |values| values[side]);
        raw.ir.blocks[arm].handlers = handlers.to_vec();
        for event in &prefix {
            let mut event = event.clone();
            match &mut event.kind {
                EventKind::Action(action) => action.map_operands(bind),
                EventKind::Define(value) => {
                    let mut source = raw.ir.values[*value].clone();
                    source.block = arm;
                    source.kind.map_operands(bind);
                    *value = raw.ir.values.push(source);
                    results.push(PhiInput {
                        block: arm,
                        throw_pc: None,
                        value: *value,
                    });
                }
                _ => (),
            }
            raw.events[arm].push(event);
        }
    }
    if let Some(value) = result {
        raw.ir.values[value].kind = ValueKind::Phi {
            inputs: results,
            slot: None,
        };
    }
    // Keep the moved instruction's precise catch snapshots on both paths.
    for value in &mut raw.ir.values {
        let ValueKind::Phi { inputs, .. } = &mut value.kind else {
            continue;
        };
        let mut moved = Vec::new();
        inputs.retain(|input| {
            if input.block != join
                || !input.throw_pc.is_some_and(|pc| {
                    prefix
                        .iter()
                        .any(|event| event.pc == pc && matches!(event.kind, EventKind::ThrowPoint))
                })
            {
                return true;
            }
            for (side, arm) in arms.into_iter().enumerate() {
                moved.push(PhiInput {
                    block: arm,
                    value: incoming
                        .get(&input.value)
                        .map_or(input.value, |values| values[side]),
                    ..input.clone()
                });
            }
            false
        });
        inputs.extend(moved);
    }
    true
}

fn recover_short_circuit(
    raw: &mut RawIr,
    predecessors: &IdVec<BlockId, Vec<BlockId>>,
    exceptional_entry: &IdVec<BlockId, bool>,
    live: &IdVec<ValueId, bool>,
) -> bool {
    for header in raw.ir.blocks.ids() {
        let Terminator::Branch {
            condition,
            then_target,
            else_target,
        } = &raw.ir.blocks[header].terminator
        else {
            continue;
        };
        let (condition, then_target, else_target) = (condition.clone(), *then_target, *else_target);
        for (tail, shared) in [(then_target, else_target), (else_target, then_target)] {
            if [header, shared].contains(&tail)
                || shared == header
                || predecessors[tail] != [header]
                || exceptional_entry[tail]
                || raw.ir.blocks[tail].handlers != raw.ir.blocks[header].handlers
            {
                continue;
            }
            let Terminator::Branch {
                condition: next,
                then_target: next_then,
                else_target: next_else,
            } = &raw.ir.blocks[tail].terminator
            else {
                continue;
            };
            let (next, next_then, next_else) = (next.clone(), *next_then, *next_else);
            let other = if next_then == shared {
                next_else
            } else if next_else == shared {
                next_then
            } else {
                continue;
            };
            if [header, tail, shared].contains(&other)
                || predecessors[other] != [tail]
                || predecessors[shared].len() != 2
                || !predecessors[shared].contains(&header)
                || !predecessors[shared].contains(&tail)
            {
                continue;
            }
            let (Terminator::Goto(join), Terminator::Goto(other_join)) = (
                &raw.ir.blocks[shared].terminator,
                &raw.ir.blocks[other].terminator,
            ) else {
                continue;
            };
            if join != other_join
                || [header, tail, shared, other].contains(join)
                || !collapsed_inputs_agree(raw, header, tail, live)
                || !stable_exception_inputs(raw, &[tail], live)
                || !region_is_expression(raw, tail, &[next.left, next.right], None, live)
            {
                continue;
            }
            let pc = raw.ir.blocks[tail].pc;
            let yes = raw.ir.values.next_id();
            raw.ir.values.push(Value {
                local: None,
                kind: ValueKind::Literal(Literal::Int(1)),
                ty: JvmType::Boolean,
                pc,
                block: header,
            });
            let no = raw.ir.values.next_id();
            raw.ir.values.push(Value {
                local: None,
                kind: ValueKind::Literal(Literal::Int(0)),
                ty: JvmType::Boolean,
                pc,
                block: header,
            });
            let next_result = raw.ir.values.next_id();
            raw.ir.values.push(Value {
                local: None,
                kind: ValueKind::Select {
                    condition: next,
                    then_value: if next_then == shared { yes } else { no },
                    else_value: if next_else == shared { yes } else { no },
                },
                ty: JvmType::Boolean,
                pc,
                block: header,
            });
            let result = raw.ir.values.next_id();
            raw.ir.values.push(Value {
                local: None,
                kind: ValueKind::Select {
                    condition,
                    then_value: if then_target == shared {
                        yes
                    } else {
                        next_result
                    },
                    else_value: if else_target == shared {
                        yes
                    } else {
                        next_result
                    },
                },
                ty: JvmType::Boolean,
                pc,
                block: header,
            });
            let mut owned = Vec::new();
            let events = std::mem::take(&mut raw.events[tail]);
            for event in &events {
                if let EventKind::Define(value) = event.kind {
                    raw.ir.values[value].block = header;
                    owned.push(value);
                }
            }
            owned.push(next_result);
            raw.events[header].extend(events);
            raw.events[header].push(Event {
                pc,
                kind: EventKind::Define(next_result),
            });
            raw.events[header].push(Event {
                pc,
                kind: EventKind::Define(result),
            });
            raw.conditional_regions
                .insert(next_result, [Vec::new(), Vec::new()]);
            raw.conditional_regions.insert(
                result,
                if then_target == tail {
                    [owned, Vec::new()]
                } else {
                    [Vec::new(), owned]
                },
            );
            raw.ir.blocks[header].terminator = Terminator::Branch {
                condition: Condition {
                    op: CompareOp::Ne,
                    left: result,
                    right: no,
                },
                then_target: shared,
                else_target: other,
            };
            raw.ir.blocks[tail].terminator = Terminator::Unreachable;
            raw.ir.blocks[tail].handlers.clear();
            for value in &mut raw.ir.values {
                if let ValueKind::Phi { inputs, .. } = &mut value.kind {
                    for input in inputs {
                        if input.block == tail {
                            input.block = header;
                        }
                    }
                }
            }
            return true;
        }
    }
    false
}

/// Collapsing two predecessor edges is valid only when every surviving phi
/// observes the same incoming frame value on those edges. Stores of existing
/// values (parameters, literals, or earlier definitions) create no Define event
/// in the tail and therefore cannot be detected by expression ownership alone.
fn collapsed_inputs_agree(
    raw: &RawIr,
    header: BlockId,
    tail: BlockId,
    live: &IdVec<ValueId, bool>,
) -> bool {
    raw.ir.values.iter_enumerated().all(|(id, value)| {
        if !live[id] {
            return true;
        }
        let ValueKind::Phi { inputs, .. } = &value.kind else {
            return true;
        };
        let mut incoming = None;
        inputs.iter().all(|input| {
            if input.throw_pc.is_some() || ![header, tail].contains(&input.block) {
                return true;
            }
            if incoming.is_some_and(|value| value != input.value) {
                return false;
            }
            incoming = Some(input.value);
            true
        })
    })
}

fn stable_exception_inputs(raw: &RawIr, arms: &[BlockId], live: &IdVec<ValueId, bool>) -> bool {
    raw.ir.values.iter_enumerated().all(|(id, value)| {
        if !live[id] {
            return true;
        }
        let ValueKind::Phi { inputs, .. } = &value.kind else {
            return true;
        };
        let mut snapshot = None;
        inputs.iter().all(|input| {
            if input.throw_pc.is_none() || !arms.contains(&input.block) {
                return true;
            }
            // The caught stack value is supplied by the exceptional edge.
            if matches!(
                raw.ir.values[input.value].kind,
                ValueKind::CaughtException { .. }
            ) {
                return true;
            }
            if arms.contains(&raw.ir.values[input.value].block)
                || snapshot.is_some_and(|old| old != input.value)
            {
                return false;
            }
            snapshot = Some(input.value);
            true
        })
    })
}

/// A Java arm is an expression DAG with one escaping result. Repeated values
/// are captured at their first use inside that arm. Compare definition identity
/// and order, including nested choices, before accepting that evaluation site.
/// NEW starts allocation/class initialization before its argument expressions;
/// its constructor invocation completes afterward. Both boundaries participate.
fn region_is_expression(
    raw: &RawIr,
    block: BlockId,
    roots: &[ValueId],
    output: Option<ValueId>,
    live: &IdVec<ValueId, bool>,
) -> bool {
    let owned: HashSet<_> = raw.events[block]
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::Define(value) => Some(value),
            _ => None,
        })
        .collect();
    let nested: HashSet<_> = raw
        .conditional_regions
        .values()
        .flatten()
        .flatten()
        .copied()
        .collect();
    let original: Vec<_> = raw.events[block]
        .iter()
        .filter_map(|event| {
            let value = match event.kind {
                EventKind::Define(value)
                | EventKind::Allocate(value)
                | EventKind::ReadTarget(value) => value,
                _ => return None,
            };
            (!nested.contains(&value))
                .then(|| evaluation::original(raw, event))
                .flatten()
        })
        .collect();
    if raw.events[block]
        .iter()
        .any(|event| matches!(event.kind, EventKind::Action(_)))
    {
        return false;
    }
    // Values used after the join, in another arm, or by an exception snapshot
    // constitute additional outputs even when the region has only one phi.
    let references_owned =
        |operands: Vec<ValueId>| operands.iter().any(|operand| owned.contains(operand));
    if raw.ir.values.iter_enumerated().any(|(id, value)| {
        if !live[id] || Some(id) == output || owned.contains(&id) {
            return false;
        }
        let mut escapes = false;
        value.kind.for_each_operand(|operand| {
            escapes = escapes || owned.contains(&operand);
        });
        escapes
    }) {
        return false;
    }
    if raw.events.iter().flatten().any(|event| {
        matches!(&event.kind, EventKind::Action(action) if references_owned(action.operands()))
    }) {
        return false;
    }
    if raw
        .ir
        .blocks
        .iter_enumerated()
        .any(|(id, source)| id != block && references_owned(source.terminator.operands()))
    {
        return false;
    }
    let mut evaluated = HashSet::new();
    let mut actual = Vec::new();
    for &root in roots {
        evaluation::expression(
            raw,
            root,
            true,
            |value, _| owned.contains(&value) && evaluated.insert(value),
            |operation| actual.push(operation),
        );
    }
    actual == original && raw.captures_dominate_reads(roots, &owned)
}
