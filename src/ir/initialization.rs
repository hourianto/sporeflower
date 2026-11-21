use super::lift::{EventKind, RawIr};
use super::*;
use crate::hash::{HashMap, HashSet};
use std::collections::BTreeMap;

/// Physical local coalescing can make a catch-visible argument an ordinary
/// assignment expression. At this point the handler reads the very same local,
/// so a throwing producer leaves its previous value intact.
pub(super) fn plan(raw: &mut RawIr) {
    let sites = requirements(raw);
    let mut changed = false;
    // Requirements are in bytecode order. Within a block, later PCs have
    // equal or later statement boundaries, so removals leave earlier sites valid.
    for site in sites.iter().rev() {
        changed |= inline_argument_bindings(raw, site);
    }
    raw.ir.initialization_sites = if changed { requirements(raw) } else { sites };
}

fn inline_argument_bindings(raw: &mut RawIr, site: &InitializationSite) -> bool {
    let ir = &raw.ir;
    let block = &ir.blocks[site.block];
    let start = site.statement_start;
    let Some(&Statement::Assign {
        local,
        value: ValueUse::Definition(rhs),
    }) = block.statements.get(start)
    else {
        return false;
    };
    // Moving a definition across NEW must not carry an earlier effect with it.
    if ir.values[rhs].local != Some(local)
        || ir.values[rhs].ty != ir.locals[local].ty
        || ir.values[rhs].pc <= site.pc
        || scheduled::values(ir, ValueUse::Definition(rhs)).operations().any(|id| {
            let value = &ir.values[id];
            value.kind.local_write().is_some()
                || ir.operation_is_observable(id) && value.pc <= site.pc
                || matches!(value.kind, ValueKind::New { allocation_pc, .. } | ValueKind::ArrayInitializer { allocation_pc, .. } if allocation_pc <= site.pc)
        })
    {
        return false;
    }
    // DUP/ASTORE can leave a stack result and a separate catch-visible slot.
    // In preservation mode both bindings survive. Nest the same-type copies
    // around their producer in original write order.
    let mut bindings = vec![local];
    for statement in block.statements[start + 1..]
        .iter()
        .take(schedule::MAX_EXPRESSION_DEPTH)
    {
        let Statement::Assign { local, value } = *statement else {
            break;
        };
        let read = match value {
            ValueUse::Definition(id) if let ValueKind::Alias(source) = ir.values[id].kind => {
                ir.values[source].local
            }
            _ => value.local(ir),
        };
        if read.is_none_or(|read| !bindings.contains(&read))
            || ir.locals[local].ty != ir.values[rhs].ty
        {
            break;
        }
        bindings.push(local);
    }
    let Some((allocation, index)) = leading_argument(ir, site, rhs, &bindings) else {
        return false;
    };
    let ir = &mut raw.ir;
    let count = bindings.len();
    let mut binding = rhs;
    for local in bindings {
        binding = ir.values.push(Value {
            kind: ValueKind::InlineAssign {
                local,
                value: binding,
            },
            ty: ir.locals[local].ty.clone(),
            local: None,
            pc: ir.values[rhs].pc,
            block: site.block,
        });
    }
    let ValueKind::New { args, .. } = &mut ir.values[allocation].kind else {
        unreachable!()
    };
    args[index] = binding;
    raw.retain_statements(site.block, |i| !(start..start + count).contains(&i));
    true
}

fn leading_argument(
    ir: &MethodIr,
    site: &InitializationSite,
    rhs: ValueId,
    bindings: &[LocalId],
) -> Option<(ValueId, usize)> {
    // Only step past independent local reads and literals. In particular, a
    // conditional operand, another NEW or a prior call can throw or skip the
    // assignment, and belongs to the ordinary allocation-region lowering.
    // Preservation may still refer to the DUPed operand, with a separate
    // alias for the slot. Both refer to the same value at this boundary.
    let alias = match ir.values[rhs].kind {
        ValueKind::Alias(source) => Some(source),
        _ => None,
    };
    let captured = |usage: ValueUse| {
        Some(usage.value()) == alias
            || usage
                .local(ir)
                .is_some_and(|local| bindings.contains(&local))
    };
    let independent = |usage: ValueUse| {
        !captured(usage)
            && (usage.local(ir).is_some()
                || matches!(ir.values[usage.value()].kind, ValueKind::Literal(_)))
    };
    let block = &ir.blocks[site.block];
    let mut first = None;
    let operand = |usage| {
        if first.is_none() && !independent(usage) {
            first = Some(usage);
        }
    };
    if let Some(statement) = block.statements.get(site.statement_start + bindings.len()) {
        scheduled::for_each_statement_root(statement, operand);
    } else {
        block
            .terminator
            .operands()
            .into_iter()
            .map(ValueUse::Reference)
            .for_each(operand);
    }
    let mut root = first?;
    loop {
        let id = root.value();
        if root.local(ir).is_some() {
            return None;
        }
        match &ir.values[id].kind {
            ValueKind::New {
                allocation_pc,
                args,
                ..
            } if *allocation_pc == site.pc => {
                let index = args
                    .iter()
                    .position(|&arg| !independent(ValueUse::Reference(arg)))?;
                return captured(ValueUse::Reference(args[index])).then_some((id, index));
            }
            ValueKind::Call { receiver, args, .. } => {
                let &first = receiver
                    .iter()
                    .chain(args)
                    .find(|&&arg| !independent(ValueUse::Reference(arg)))?;
                root = ValueUse::Reference(first);
            }
            ValueKind::Cast { value, .. }
            | ValueKind::Field {
                receiver: Some(value),
                ..
            } => root = ValueUse::Reference(*value),
            _ => return None,
        }
    }
}

/// Compare source evaluation boundaries with the original NEW identity. The
/// scheduler has already recovered compound expressions and captures; only
/// operations still preceding a Java NEW can require an explicit trigger.
pub(super) fn requirements(raw: &RawIr) -> Vec<InitializationSite> {
    let ir = &raw.ir;
    let handled: HashSet<_> = ir
        .values
        .iter()
        .filter_map(|value| match &value.kind {
            ValueKind::InitializeClass { class } => Some((value.pc, class.as_str())),
            _ => None,
        })
        .collect();
    let mut allocations = BTreeMap::new();
    for value in &ir.values {
        let ValueKind::Allocation { class } = &value.kind else {
            continue;
        };
        let key = (value.pc, class.as_str());
        if !handled.contains(&key) {
            let pcs = raw.statement_pcs(value.block);
            allocations.insert(
                value.pc,
                Allocation {
                    class,
                    block: value.block,
                    statement_start: pcs
                        .iter()
                        .position(|&pc| pc > value.pc)
                        .unwrap_or(pcs.len()),
                    seen: false,
                    displaced: false,
                    observed: Vec::new(),
                },
            );
        }
    }
    if allocations.is_empty() {
        return vec![];
    }

    // A pure store is observable at NEW when its handler consumes that slot.
    // Use the exact throw-PC inputs, so unrelated temporaries never force an
    // allocation trigger. Record each store once even when many allocations
    // in the same block share that catch.
    let mut writes = HashMap::default();
    if allocations
        .values()
        .any(|a| !ir.blocks[a.block].handlers.is_empty())
    {
        for value in &ir.values {
            let (Some(local), ValueKind::Phi { inputs, .. }) = (value.local, &value.kind) else {
                continue;
            };
            for input in inputs {
                if let Some(site) = input.throw_pc.and_then(|pc| allocations.get_mut(&pc))
                    && site.block == input.block
                {
                    site.observed.push(local);
                    writes.insert((site.block, local), 0);
                }
            }
        }
    }
    let mut order = AllocationOrder {
        ir,
        allocations,
        writes,
    };
    for (block_id, block) in ir.blocks.iter_enumerated() {
        let mut action_pcs = raw
            .events
            .get(block_id)
            .into_iter()
            .flatten()
            .filter_map(|event| matches!(event.kind, EventKind::Action(_)).then_some(event.pc));
        let mut latest = None;
        for (index, statement) in block.statements.iter().enumerate() {
            scheduled::for_each_statement_root(statement, |usage| {
                order.visit(usage, block_id, &mut latest);
            });
            if let Statement::Assign { local, .. } = statement
                && let Some(latest) = order.writes.get_mut(&(block_id, *local))
            {
                let written = raw.statement_pcs(block_id)[index];
                if written != u32::MAX {
                    *latest = written.max(*latest);
                }
            }
            if !matches!(statement, Statement::Assign { .. } | Statement::Eval(_))
                && let Some(pc) = action_pcs.next()
            {
                record(&mut latest, pc);
            }
        }
        for value in block.terminator.operands() {
            order.visit(ValueUse::Reference(value), block_id, &mut latest);
        }
    }
    order
        .allocations
        .into_iter()
        .filter(|(_, allocation)| !allocation.seen || allocation.displaced)
        .map(|(pc, allocation)| InitializationSite {
            class: allocation.class.to_owned(),
            pc,
            block: allocation.block,
            statement_start: allocation.statement_start,
        })
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

struct Allocation<'a> {
    class: &'a str,
    block: BlockId,
    statement_start: usize,
    seen: bool,
    displaced: bool,
    observed: Vec<LocalId>,
}

struct AllocationOrder<'a> {
    ir: &'a MethodIr,
    allocations: BTreeMap<u32, Allocation<'a>>,
    writes: HashMap<(BlockId, LocalId), u32>,
}

impl AllocationOrder<'_> {
    fn visit(&mut self, usage: ValueUse, block: BlockId, latest: &mut Option<u32>) {
        if usage.local(self.ir).is_some() {
            return;
        }
        let source = &self.ir.values[usage.value()];
        match &source.kind {
            ValueKind::New { allocation_pc, .. } => {
                if let Some(allocation) = self.allocations.get_mut(allocation_pc) {
                    allocation.seen = true;
                    allocation.displaced |= allocation.block != block
                        || latest.is_some_and(|pc| pc > *allocation_pc)
                        || allocation
                            .observed
                            .iter()
                            .any(|&local| self.writes[&(allocation.block, local)] > *allocation_pc);
                    record(latest, *allocation_pc);
                }
            }
            ValueKind::ArrayInitializer { allocation_pc, .. } => record(latest, *allocation_pc),
            ValueKind::Phi { .. }
            | ValueKind::Parameter { .. }
            | ValueKind::CaughtException { .. }
            | ValueKind::Allocation { .. } => return,
            ValueKind::Select {
                condition,
                then_value,
                else_value,
            } => {
                for operand in [condition.left, condition.right] {
                    self.visit(ValueUse::Reference(operand), block, latest);
                }
                // Both arms start after the condition, independently of each other.
                let mut other = *latest;
                self.visit(ValueUse::Reference(*then_value), block, latest);
                self.visit(ValueUse::Reference(*else_value), block, &mut other);
                if let Some(pc) = other {
                    record(latest, pc);
                }
                return;
            }
            _ => {}
        }
        usage.for_each_child(self.ir, |operand| self.visit(operand, block, latest));
        if source.kind.is_observable() && source.kind.local_write().is_none() {
            record(latest, source.pc);
        }
    }
}
