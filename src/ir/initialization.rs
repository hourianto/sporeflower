use super::lift::{EventKind, RawIr};
use super::*;
use crate::hash::HashSet;
use std::collections::BTreeMap;

/// Compare source evaluation boundaries with the original NEW identity. The
/// scheduler has already recovered compound expressions and captures; only
/// operations still preceding a Java NEW can require an explicit trigger.
pub(super) fn requirements(raw: &RawIr) -> Vec<InitializationSite> {
    let mut allocations = BTreeMap::new();
    let handled: HashSet<_> = raw
        .ir
        .values
        .iter()
        .filter_map(|value| {
            if let ValueKind::InitializeClass { class } = &value.kind {
                Some((value.pc, class.clone()))
            } else {
                None
            }
        })
        .collect();
    for value in &raw.ir.values {
        if let ValueKind::Allocation { class } = &value.kind {
            if handled.contains(&(value.pc, class.clone())) {
                continue;
            }
            let statement_start = raw
                .statement_pcs(value.block)
                .iter()
                .position(|&pc| pc > value.pc)
                .unwrap_or(raw.ir.blocks[value.block].statements.len());
            allocations.insert(
                (value.pc, class.clone()),
                InitializationSite {
                    class: class.clone(),
                    pc: value.pc,
                    block: value.block,
                    statement_start,
                },
            );
        }
    }
    if allocations.is_empty() {
        return vec![];
    }
    let mut seen = HashSet::default();
    let mut needed = HashSet::default();
    for (block_id, block) in raw.ir.blocks.iter_enumerated() {
        let action_pcs: Vec<_> = raw
            .events
            .get(block_id)
            .into_iter()
            .flatten()
            .filter_map(|event| matches!(event.kind, EventKind::Action(_)).then_some(event.pc))
            .collect();
        let mut action = 0;
        let mut latest = None;
        for statement in &block.statements {
            match statement {
                Statement::Assign { value, .. } => visit(
                    &raw.ir,
                    *value,
                    block_id,
                    &allocations,
                    &mut seen,
                    &mut needed,
                    &mut latest,
                ),
                Statement::Eval(value) => visit(
                    &raw.ir,
                    ValueUse::Definition(*value),
                    block_id,
                    &allocations,
                    &mut seen,
                    &mut needed,
                    &mut latest,
                ),
                _ => {
                    for value in statement.operands() {
                        visit(
                            &raw.ir,
                            ValueUse::Reference(value),
                            block_id,
                            &allocations,
                            &mut seen,
                            &mut needed,
                            &mut latest,
                        );
                    }
                    if let Some(&pc) = action_pcs.get(action) {
                        record(&mut latest, pc);
                    }
                    action += 1;
                }
            }
        }
        for value in block.terminator.operands() {
            visit(
                &raw.ir,
                ValueUse::Reference(value),
                block_id,
                &allocations,
                &mut seen,
                &mut needed,
                &mut latest,
            );
        }
    }
    allocations
        .into_iter()
        .filter_map(|(key, site)| (!seen.contains(&key) || needed.contains(&key)).then_some(site))
        .collect()
}

pub(super) fn insert_triggers(
    raw: &mut RawIr,
    sites: &[InitializationSite],
    instructions: &[Instruction],
    cfg: &ControlFlowGraph,
) -> Result<()> {
    for site in sites {
        // The discovery pass may have merged NEW's scheduled block into an
        // earlier select header. Replay precedes that recovery, so only the
        // normalized instruction PC identifies the original insertion point.
        let owner = cfg.block_at_pc(instructions, site.pc).ok_or_else(|| {
            anyhow::anyhow!(
                "planned initialization instruction at {} no longer exists",
                site.pc
            )
        })?;
        let block = raw
            .events
            .get_mut(owner)
            .ok_or_else(|| anyhow::anyhow!("planned initialization block no longer exists"))?;
        let position = block
            .iter()
            .position(|event| {
                let EventKind::Define(value) = event.kind else {
                    return false;
                };
                if raw.ir.values[value].pc != site.pc {
                    return false;
                }
                match &raw.ir.values[value].kind {
                    ValueKind::Allocation { class } => *class == site.class,
                    ValueKind::Field { receiver: None, .. } => true,
                    _ => false,
                }
            })
            .ok_or_else(|| {
                anyhow::anyhow!("planned initialization at {} no longer exists", site.pc)
            })?;
        let value = raw.ir.values.next_id();
        raw.ir.values.push(Value {
            local: None,
            kind: ValueKind::InitializeClass {
                class: site.class.clone(),
            },
            ty: JvmType::Void,
            pc: site.pc,
            block: owner,
        });
        // The original ThrowPoint immediately precedes this position. Its local
        // snapshots therefore cover initialization failures, as for NEW/GETSTATIC.
        block.insert(
            position,
            super::lift::Event {
                pc: site.pc,
                kind: EventKind::Define(value),
            },
        );
    }
    Ok(())
}

fn record(latest: &mut Option<u32>, pc: u32) {
    *latest = Some(latest.map_or(pc, |old| old.max(pc)));
}

#[allow(clippy::too_many_arguments)]
fn visit(
    ir: &MethodIr,
    usage: ValueUse,
    block: BlockId,
    allocations: &BTreeMap<(u32, String), InitializationSite>,
    seen: &mut HashSet<(u32, String)>,
    needed: &mut HashSet<(u32, String)>,
    latest: &mut Option<u32>,
) {
    if usage.local(ir).is_some() {
        return;
    }
    let value = usage.value();
    let source = &ir.values[value];
    if let ValueKind::New {
        class,
        allocation_pc,
        ..
    } = &source.kind
    {
        let key = (*allocation_pc, class.clone());
        if let Some(allocation) = allocations.get(&key) {
            seen.insert(key.clone());
            if allocation.block != block || latest.is_some_and(|pc| pc > allocation.pc) {
                needed.insert(key);
            }
            record(latest, allocation.pc);
        }
    }
    if let ValueKind::ArrayInitializer { allocation_pc, .. } = &source.kind {
        record(latest, *allocation_pc);
    }
    if matches!(
        source.kind,
        ValueKind::Phi { .. }
            | ValueKind::Parameter { .. }
            | ValueKind::CaughtException { .. }
            | ValueKind::Allocation { .. }
    ) {
        return;
    }
    if let ValueKind::Select {
        condition,
        then_value,
        else_value,
    } = &source.kind
    {
        for operand in [condition.left, condition.right] {
            visit(
                ir,
                ValueUse::Reference(operand),
                block,
                allocations,
                seen,
                needed,
                latest,
            );
        }
        // Alternative allocations do not precede each other. Both arms start
        // from the condition's state, then contribute possible later effects.
        let mut other = *latest;
        visit(
            ir,
            ValueUse::Reference(*then_value),
            block,
            allocations,
            seen,
            needed,
            latest,
        );
        visit(
            ir,
            ValueUse::Reference(*else_value),
            block,
            allocations,
            seen,
            needed,
            &mut other,
        );
        if let Some(pc) = other {
            record(latest, pc);
        }
        return;
    }
    usage.for_each_child(ir, |operand| {
        visit(ir, operand, block, allocations, seen, needed, latest)
    });
    if source.kind.is_observable() && source.kind.local_write().is_none() {
        record(latest, source.pc);
    }
}
