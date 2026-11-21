use super::*;

/// Physical local coalescing can make a catch-visible argument an ordinary
/// assignment expression. At this point the handler reads the very same local,
/// so a throwing producer leaves its previous value intact.
pub(super) fn inline(raw: &mut RawIr, sites: &[InitializationSite]) -> bool {
    let mut changed = false;
    let mut spills = None;
    // Requirements are in bytecode order. Within a block, later PCs have
    // equal or later statement boundaries, so removals leave earlier sites valid.
    for site in sites.iter().rev() {
        // Scheduling can spill several arguments before the same NEW. Start
        // at the consumer and recover their bindings in reverse statement
        // order; each substitution still evaluates at its original operand.
        let start = site.statement_start;
        if inline_argument_bindings(raw, site, start, &mut spills) {
            changed = true;
            continue;
        }
        let count = raw.ir.blocks[site.block].statements[start..]
            .iter()
            .take(schedule::MAX_EXPRESSION_DEPTH)
            .take_while(|stmt| matches!(stmt, Statement::Assign { .. }))
            .count();
        for index in (start..start + count).rev() {
            changed |= inline_argument_bindings(raw, site, index, &mut spills);
        }
    }
    changed
}

fn single_use_spills(raw: &RawIr) -> HashSet<LocalId> {
    let (reads, writes) = schedule::local_access_counts(&raw.ir);
    let mut spills: HashSet<_> = raw
        .ir
        .locals
        .ids()
        .filter(|&id| {
            let local = &raw.ir.locals[id];
            reads[id] == 1
                && writes[id] == 1
                && local.parameter.is_none()
                && !local.is_this
                && !local.needs_default
        })
        .collect();
    for access in &raw.local_accesses {
        if access.store
            && let Some(local) = raw.ir.values[access.value].local
        {
            spills.remove(&local);
        }
    }
    for &value in &raw.retained {
        if let Some(local) = raw.ir.values[value].local {
            spills.remove(&local);
        }
    }
    spills
}

fn inline_argument_bindings(
    raw: &mut RawIr,
    site: &InitializationSite,
    start: usize,
    spills: &mut Option<HashSet<LocalId>>,
) -> bool {
    let ir = &raw.ir;
    let block = &ir.blocks[site.block];
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
    let Some(ArgumentUse { path, read }) =
        argument_binding_path(ir, site, start + bindings.len(), rhs, &bindings)
    else {
        return false;
    };
    let count = bindings.len();
    let drop_spill = count == 1
        && read == Some(local)
        && spills
            .get_or_insert_with(|| single_use_spills(raw))
            .contains(&local);
    let ir = &mut raw.ir;
    let mut binding = rhs;
    if drop_spill {
        // The scheduler supplied this binding solely to hold an argument on
        // the stack across another store. Its only read is being replaced.
        ir.values[rhs].local = None;
    } else {
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
    }
    // Only rewrite this occurrence. Other expressions may reuse a pure
    // subtree without owning the catch-visible assignment.
    let (&(allocation, index), nested) = path.split_first().unwrap();
    for &(parent, index) in nested.iter().rev() {
        let mut value = ir.values[parent].clone();
        let mut position = 0;
        value.kind.map_operands(|operand| {
            let replaced = if position == index { binding } else { operand };
            position += 1;
            replaced
        });
        binding = ir.values.push(value);
    }
    let ValueKind::New { args, .. } = &mut ir.values[allocation].kind else {
        unreachable!()
    };
    args[index] = binding;
    raw.retain_statements(site.block, |i| !(start..start + count).contains(&i));
    true
}

struct ArgumentUse {
    path: Vec<(ValueId, usize)>,
    read: Option<LocalId>,
}

fn argument_binding_path(
    ir: &MethodIr,
    site: &InitializationSite,
    consumer: usize,
    rhs: ValueId,
    bindings: &[LocalId],
) -> Option<ArgumentUse> {
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
    if let Some(statement) = block.statements.get(consumer) {
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
    let mut path = Vec::new();
    for _ in 0..schedule::MAX_EXPRESSION_DEPTH {
        let id = root.value();
        if captured(root) {
            return (!path.is_empty()).then(|| ArgumentUse {
                path,
                read: root.local(ir),
            });
        }
        if root.local(ir).is_some() {
            return None;
        }
        // Most operations evaluate every operand left-to-right before doing
        // their own work. A NEW starts before its arguments; only cross one
        // whose class initialization already preceded the original binding.
        let at_allocation = match ir.values[id].kind {
            ValueKind::New { allocation_pc, .. } if allocation_pc <= site.pc => {
                allocation_pc == site.pc
            }
            ValueKind::New { .. }
            | ValueKind::ArrayInitializer { .. }
            | ValueKind::Select { .. }
            | ValueKind::CompoundUpdate { .. }
            | ValueKind::LocalUpdate { .. } => return None,
            _ => false,
        };
        let mut next = None;
        let mut index = 0;
        root.for_each_child(ir, |child| {
            if next.is_none() && !independent(child) {
                next = Some((index, child));
            }
            index += 1;
        });
        let (index, child) = next?;
        if at_allocation || !path.is_empty() {
            path.push((id, index));
        }
        root = child;
    }
    None
}
