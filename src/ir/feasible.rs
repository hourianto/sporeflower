//! Remove CFG edges decided by literals or certified normal call results.
//! Original call definitions and throw points stay scheduled even if the result
//! becomes unused. No fact here certifies purity or successful completion.
use super::lift::{EventKind, RawIr};
use super::*;
use crate::index::{BlockId, IdVec};

pub(super) fn simplify(
    raw: &mut RawIr,
    instructions: &[Instruction],
    cfg: &ControlFlowGraph,
    normal_return: Option<NormalIntReturn<'_>>,
) {
    let access_blocks: Vec<_> = raw
        .local_accesses
        .iter()
        .map(|access| {
            cfg.block_at_pc(instructions, access.pc)
                .expect("local access belongs to a decoded instruction")
        })
        .collect();
    let mut access_blocks = access_blocks;
    loop {
        let mut changed = false;
        for block in &mut raw.ir.blocks {
            let target = match &block.terminator {
                Terminator::Branch {
                    condition,
                    then_target,
                    else_target,
                } => compare(
                    &raw.ir.values[condition.left].kind,
                    &raw.ir.values[condition.right].kind,
                    condition.op,
                    normal_return,
                )
                .map(|yes| if yes { *then_target } else { *else_target }),
                Terminator::Switch {
                    value,
                    cases,
                    default,
                } => integer(&raw.ir.values[*value].kind, normal_return).map(|number| {
                    cases
                        .iter()
                        .find(|(key, _)| *key == number)
                        .map_or(*default, |(_, target)| *target)
                }),
                _ => None,
            };
            if let Some(target) = target {
                block.terminator = Terminator::Goto(target);
                changed = true;
            }
        }
        let throws: IdVec<BlockId, Vec<_>> = raw
            .events
            .iter()
            .map(|events| {
                events
                    .iter()
                    .filter_map(|event| {
                        matches!(event.kind, EventKind::ThrowPoint).then_some(event.pc)
                    })
                    .collect()
            })
            .collect();
        let handlers: IdVec<BlockId, Vec<_>> = raw
            .ir
            .blocks
            .iter_enumerated()
            .map(|(block, source)| {
                if throws[block].is_empty() {
                    Vec::new()
                } else {
                    crate::cfg::dispatch::effective(&source.handlers)
                        .map(|handler| handler.target)
                        .collect()
                }
            })
            .collect();
        let mut reachable: IdVec<BlockId, _> = vec![false; raw.ir.blocks.len()].into();
        let mut pending = vec![raw.ir.entry];
        while let Some(block) = pending.pop() {
            if std::mem::replace(&mut reachable[block], true) {
                continue;
            }
            pending.extend(raw.ir.blocks[block].terminator.successors());
            pending.extend(&handlers[block]);
        }
        let successors: IdVec<BlockId, _> = raw
            .ir
            .blocks
            .iter()
            .map(|block| block.terminator.successors())
            .collect();
        for value in &mut raw.ir.values {
            if let ValueKind::Phi { inputs, .. } = &mut value.kind {
                let before = inputs.len();
                inputs.retain(|input| {
                    reachable[input.block]
                        && if let Some(pc) = input.throw_pc {
                            throws[input.block].binary_search(&pc).is_ok()
                                && handlers[input.block].contains(&value.block)
                        } else {
                            successors[input.block].contains(&value.block)
                        }
                });
                changed |= before != inputs.len();
            }
        }
        for (block, live) in reachable.iter_enumerated().map(|(id, &live)| (id, live)) {
            if live {
                // Shadowed entries still describe the lexical scope around
                // inner catches. Executable reachability must not erase them.
                continue;
            }
            changed |= !matches!(raw.ir.blocks[block].terminator, Terminator::Unreachable)
                || !raw.events[block].is_empty();
            raw.ir.blocks[block].terminator = Terminator::Unreachable;
            raw.ir.blocks[block].handlers.clear();
            raw.set_statements(block, vec![]);
            raw.ir.blocks[block].caught_exception = None;
            raw.events[block].clear();
        }
        // A shared handler's original type may have included a now-dead catch
        // edge. Its entry value is typed by the dispatch edges that still exist.
        let mut caught_types: IdVec<BlockId, _> = vec![None; raw.ir.blocks.len()].into();
        for (block, live) in reachable.iter_enumerated().map(|(id, &live)| (id, live)) {
            if !live || throws[block].is_empty() {
                continue;
            }
            for edge in crate::cfg::dispatch::effective(&raw.ir.blocks[block].handlers) {
                let incoming = JvmType::Object(
                    edge.catch_type
                        .clone()
                        .unwrap_or_else(|| "java/lang/Throwable".into()),
                );
                let fact = &mut caught_types[edge.target];
                if fact.as_ref().is_some_and(|existing| *existing != incoming) {
                    *fact = Some(JvmType::Object("java/lang/Throwable".into()));
                } else {
                    *fact = Some(incoming);
                }
            }
        }
        for (handler, block) in raw.ir.blocks.iter_enumerated_mut() {
            let caught = &mut block.caught_exception;
            if let Some(value) = *caught {
                if let Some(ty) = &caught_types[handler] {
                    raw.ir.values[value].ty = ty.clone();
                } else {
                    *caught = None;
                }
            }
        }
        // Dead definition arenas retain their IDs, but no longer contribute
        // effects, allocation requirements, or provisional source type facts.
        for value in &mut raw.ir.values {
            if !reachable[value.block] {
                value.kind = ValueKind::Phi {
                    inputs: vec![],
                    slot: None,
                };
            }
        }
        let mut index = 0;
        raw.local_accesses.retain(|_| {
            let live = reachable[access_blocks[index]];
            index += 1;
            live
        });
        access_blocks.retain(|&block| reachable[block]);
        if !changed {
            break;
        }
        super::lift::canonicalize(raw);
    }
}

fn integer(value: &ValueKind, normal_return: Option<NormalIntReturn<'_>>) -> Option<i32> {
    match value {
        ValueKind::Literal(Literal::Int(value)) => Some(*value),
        ValueKind::Call {
            kind: InvokeKind::Static,
            method,
            ..
        } => normal_return?(method),
        _ => None,
    }
}

fn compare(
    left: &ValueKind,
    right: &ValueKind,
    op: CompareOp,
    normal_return: Option<NormalIntReturn<'_>>,
) -> Option<bool> {
    let order = if let (Some(left), Some(right)) =
        (integer(left, normal_return), integer(right, normal_return))
    {
        left.cmp(&right)
    } else {
        match (left, right) {
            (ValueKind::Literal(Literal::Null), ValueKind::Literal(Literal::Null))
                if matches!(op, CompareOp::Eq | CompareOp::Ne) =>
            {
                std::cmp::Ordering::Equal
            }
            _ => return None,
        }
    };
    Some(op.test(Some(order)))
}
