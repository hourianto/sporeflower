use super::evaluation::{self, Operation};
use super::lift::{EventKind, RawIr};
use super::scheduled::{self, ValueUse};
use super::*;
use crate::index::IdVec;
use anyhow::{bail, ensure};
use std::collections::{BTreeMap, HashMap};

const MAX_EXPRESSION_DEPTH: usize = 48;
type SnapshotCopies = HashMap<(BlockId, u32), Vec<(ValueId, ValueId)>>;

pub(super) fn schedule(
    mut raw: RawIr,
    local_variables: &[crate::classfile::LocalVariable],
    debug_pcs: Option<&[(u32, u32)]>,
    reference_assignable: Option<ReferenceAssignable<'_>>,
) -> Result<MethodIr> {
    let count = raw.ir.values.len();
    let conditional_values: std::collections::HashSet<_> = raw
        .conditional_regions
        .values()
        .flatten()
        .flatten()
        .copied()
        .collect();
    let used = live_values(&raw);
    let definitions = super::defaults::definitions(&raw, &used);
    let snapshots = snapshot_copies(&raw, &used);
    let mut uses: IdVec<ValueId, _> = vec![0usize; count].into();
    let mut external: IdVec<ValueId, _> = vec![false; count].into();
    for (id, value) in raw.ir.values.iter_enumerated() {
        if !used[id] {
            continue;
        }
        match &value.kind {
            ValueKind::Phi { inputs, .. } => {
                for input in inputs {
                    if let Some(pc) = input.throw_pc
                        && !snapshots
                            .get(&(input.block, pc))
                            .is_some_and(|copies| copies.contains(&(id, input.value)))
                    {
                        continue;
                    }
                    uses[input.value] += 1;
                    // Edge assignments execute in the predecessor. Their operands
                    // must already exist, including for exceptional predecessors.
                    if raw.ir.values[input.value].block != input.block
                        || input.throw_pc.is_some()
                        || raw.ir.blocks[input.block].terminator.successors().len() != 1
                    {
                        external[input.value] = true;
                    }
                }
            }
            _ => value.kind.for_each_operand(|operand| {
                uses[operand] += 1;
                if raw.ir.values[operand].block != value.block {
                    external[operand] = true;
                }
            }),
        }
    }
    for (block, events) in raw.events.iter_enumerated() {
        for event in events {
            if let EventKind::Action(action) = &event.kind {
                for value in action.operands() {
                    uses[value] += 1;
                    if raw.ir.values[value].block != block {
                        external[value] = true;
                    }
                }
            }
        }
        for value in raw.ir.blocks[block].terminator.operands() {
            uses[value] += 1;
            if raw.ir.values[value].block != block {
                external[value] = true;
            }
        }
    }
    debug_assert!(raw.ir.values.iter().all(|value| value.local.is_none()));
    let mut materialize: IdVec<ValueId, _> = vec![false; count].into();
    // Signature bindings exist even when no operation reads the parameter.
    for (id, value) in raw.ir.values.iter_enumerated() {
        if !used[id] && !matches!(value.kind, ValueKind::Parameter { .. }) {
            continue;
        }
        materialize[id] = match &value.kind {
            ValueKind::Parameter { .. }
            | ValueKind::CaughtException { .. }
            | ValueKind::Phi { .. } => true,
            ValueKind::Literal(_) => definitions.contains_key(&id),
            ValueKind::Allocation { .. } => false,
            _ => {
                uses[id] > 1
                    || external[id]
                    || (uses[id] == 0 && !statement_expression(&value.kind))
            }
        };
    }
    // Parallel-copy ordering must not reorder observable evaluation. Capture an
    // incoming expression as one unit, retaining its nested call/argument tree;
    // capturing every descendant separately needlessly destroys source expressions.
    let mut edge_sources = Vec::new();
    for (id, value) in raw.ir.values.iter_enumerated() {
        if used[id]
            && let ValueKind::Phi { inputs, .. } = &value.kind
        {
            edge_sources.extend(inputs.iter().map(|input| input.value));
        }
    }
    for value in edge_sources {
        if !materialize[value] && contains_inline_effect(&raw.ir, value, &materialize) {
            materialize[value] = true;
        }
    }
    // An allocation spanning control flow needs an explicit initialization
    // boundary. Source argument-region extraction moves that whole definition;
    // nesting it inside a later invocation would hide independent operands that
    // must remain before INIT. The order proof below pins those operands normally.
    let allocation_blocks: HashMap<_, _> = raw
        .ir
        .values
        .iter()
        .filter_map(|value| match &value.kind {
            ValueKind::Allocation { .. } => Some((value.pc, value.block)),
            _ => None,
        })
        .collect();
    for (id, value) in raw.ir.values.iter_enumerated() {
        if let ValueKind::New { allocation_pc, .. } = &value.kind
            && allocation_blocks
                .get(allocation_pc)
                .is_some_and(|&block| block != value.block)
        {
            materialize[id] = true;
        }
    }
    // First find the expression boundaries required by evaluation order. Depth
    // is a property of those source trees, not the original SSA dependency DAG:
    // measuring it earlier can split a later expression solely because it uses
    // an earlier expression which will already be stored in a local.
    let order = EvaluationOrder::new(&raw, &used, &uses, &snapshots, &conditional_values);
    order.enforce(&mut materialize);
    // Stable arena order is bytecode definition order inside a block, so operand
    // depths are available as we visit definitions; phis and parameters stop it.
    let mut depths: IdVec<ValueId, _> = vec![0; count].into();
    for events in &raw.events {
        for event in events {
            if let EventKind::Define(value) = event.kind {
                let depth = raw.ir.values[value]
                    .kind
                    .operands()
                    .into_iter()
                    .map(|v| depths[v])
                    .max()
                    .unwrap_or(0)
                    + 1;
                if depth >= MAX_EXPRESSION_DEPTH {
                    materialize[value] = true;
                }
                depths[value] = if materialize[value] { 0 } else { depth };
            }
        }
    }
    // Embedding an expression is valid exactly when Java evaluates observable
    // definitions in their original order. Pin only definitions participating in
    // an inversion; ordinary nested calls remain compact expression trees.
    order.enforce(&mut materialize);
    drop(order);

    // A void invocation is already an anchored statement; it has no value that
    // could be captured even when a neighboring expression must be pinned.
    for (id, value) in raw.ir.values.iter_enumerated() {
        if value.ty == JvmType::Void
            || !used[id] && !matches!(value.kind, ValueKind::Parameter { .. })
        {
            materialize[id] = false;
        }
    }

    for id in raw.ir.values.ids().take(count) {
        if !materialize[id] {
            continue;
        }
        let value = &raw.ir.values[id];
        let (name, parameter, is_this, needs_default) = match &value.kind {
            ValueKind::Parameter { slot, index: None } => {
                ("this".to_owned(), None, *slot == 0, false)
            }
            ValueKind::Parameter {
                slot,
                index: Some(index),
            } => (format!("p{slot}"), Some(*index), false, false),
            ValueKind::CaughtException { handler } => (format!("e{handler}"), None, false, false),
            ValueKind::Phi { inputs, slot } => (
                format!("v{}", raw.ir.locals.len()),
                None,
                false,
                slot.is_some() && inputs.iter().any(|i| i.throw_pc.is_some()),
            ),
            _ => (format!("v{}", raw.ir.locals.len()), None, false, false),
        };
        let local = raw.ir.locals.next_id();
        raw.ir.locals.push(Local {
            name_hint: None,
            name,
            ty: definitions.get(&id).unwrap_or(&value.ty).clone(),
            parameter,
            is_this,
            needs_default,
        });
        raw.ir.values[id].local = Some(local);
    }
    // The exception-stack value is delivered by the catch edge, after the
    // throwing operation. Bind it directly into a mixed normal/exception stack
    // phi's source local; normal predecessors already copy into the same local.
    // It must never be captured as if it existed in the prethrow local frame.
    let mut catch_bindings = Vec::new();
    for (id, value) in raw.ir.values.iter_enumerated() {
        if let ValueKind::Phi { inputs, slot: None } = &value.kind
            && let Some(local) = raw.ir.values[id].local
        {
            for input in inputs {
                if exception_stack_input(&raw.ir, value.block, input) {
                    catch_bindings.push((input.value, local));
                }
            }
        }
    }
    for (value, local) in catch_bindings {
        raw.ir.values[value].local = Some(local);
    }
    let captured = capture_conditional_values(&mut raw, &uses);
    capture_constructor_arguments(&mut raw, &used, &uses, captured, &allocation_blocks);

    // Phi assignments belong to edges, never to a successor's statement list.
    // Exception edges use snapshots immediately before the throwing instruction.
    let mut normal_copies: BTreeMap<(BlockId, BlockId), Vec<(LocalId, ValueId)>> = BTreeMap::new();
    let mut exception_copies: HashMap<(BlockId, u32), Vec<(LocalId, ValueId)>> = HashMap::new();
    for id in raw.ir.values.ids().take(count) {
        if !used[id] {
            continue;
        }
        let ValueKind::Phi { inputs, .. } = &raw.ir.values[id].kind else {
            continue;
        };
        let Some(local) = raw.ir.values[id].local else {
            continue;
        };
        for input in inputs {
            if raw.ir.values[input.value].local == Some(local) {
                continue;
            }
            if input.throw_pc.is_none() {
                normal_copies
                    .entry((input.block, raw.ir.values[id].block))
                    .or_default()
                    .push((local, input.value));
            }
        }
    }
    for (point, copies) in &snapshots {
        for &(phi, value) in copies {
            let Some(local) = raw.ir.values[phi].local else {
                continue;
            };
            if raw.ir.values[value].local != Some(local) {
                exception_copies
                    .entry(*point)
                    .or_default()
                    .push((local, value));
            }
        }
    }

    let source_slots = super::local_identity::SourceSlots::new(&raw.ir);
    for block in raw.events.ids() {
        let mut statements = vec![];
        for event in &raw.events[block] {
            match &event.kind {
                EventKind::ThrowPoint => {
                    if let Some(copies) = exception_copies.remove(&(block, event.pc)) {
                        statements.extend(
                            parallel_copies(&mut raw.ir, block, event.pc, copies, &source_slots)
                                .into_iter()
                                .map(|statement| (statement, event.pc)),
                        );
                    }
                }
                EventKind::Define(id) => {
                    if !used[*id]
                        || conditional_values.contains(id)
                        || matches!(raw.ir.values[*id].kind, ValueKind::Allocation { .. })
                    {
                        continue;
                    }
                    if let Some(local) = raw.ir.values[*id].local {
                        statements.push((
                            Statement::Assign {
                                local,
                                value: ValueUse::Definition(*id),
                            },
                            event.pc,
                        ));
                    } else if uses[*id] == 0 && raw.ir.operation_is_observable(*id) {
                        statements.push((Statement::Eval(*id), event.pc));
                    }
                }
                EventKind::Action(action) => statements.push((action.clone(), event.pc)),
                EventKind::Allocate(_) | EventKind::ReadTarget(_) => {}
            }
        }
        raw.set_statements(block, statements);
    }
    ensure!(
        exception_copies.is_empty(),
        "missing exception snapshot point"
    );

    for ((source, target), copies) in normal_copies {
        let pc = raw.ir.blocks[source].pc;
        let copies = parallel_copies(&mut raw.ir, source, pc, copies, &source_slots);
        if copies.is_empty() {
            continue;
        }
        if matches!(raw.ir.blocks[source].terminator, Terminator::Goto(_)) {
            raw.extend_statements(
                source,
                copies.into_iter().map(|statement| (statement, u32::MAX)),
            );
        } else {
            let edge = raw.push_edge(source, target);
            raw.extend_statements(
                edge,
                copies.into_iter().map(|statement| (statement, u32::MAX)),
            );
            raw.ir.blocks[source].terminator.redirect(target, edge);
        }
    }
    for (id, live) in used.iter_enumerated() {
        if *live && matches!(raw.ir.values[id].kind, ValueKind::Allocation { .. }) {
            bail!(
                "uninitialized allocation remains a live Java value at bytecode {}",
                raw.ir.values[id].pc
            );
        }
    }
    split_monitor_copies(&mut raw);
    super::defaults::hoist(&mut raw);
    super::debug_names::assign(&mut raw, local_variables, debug_pcs);
    super::local_updates::recover(&mut raw);
    coalesce_adjacent_copies(&mut raw, reference_assignable);
    super::available_copies::eliminate(&mut raw);
    super::type_facts::refine_integral_locals(&mut raw.ir);
    if super::coalesce::run(&mut raw) != 0 {
        // All copy facts refer to pre-merge storage and exact statement indices.
        // Recompute them over the transformed physical local graph.
        super::available_copies::eliminate_after_coalescing(&mut raw);
    }
    super::debug_names::assign_final(&mut raw, local_variables, debug_pcs);
    raw.assert_statement_origins();
    raw.ir.initialization_sites = super::initialization::requirements(&raw);
    Ok(raw.ir)
}

/// Write a single-use expression result directly into the following copy's
/// destination. RHS evaluation and its exception boundary are unchanged; a
/// destination is committed only after the complete expression succeeds.
fn coalesce_adjacent_copies(
    raw: &mut RawIr,
    reference_assignable: Option<ReferenceAssignable<'_>>,
) {
    let ir = &raw.ir;
    let mut reads: IdVec<LocalId, _> = vec![0usize; ir.locals.len()].into();
    let mut definitions: IdVec<LocalId, _> = vec![0usize; ir.locals.len()].into();
    for block in &ir.blocks {
        for statement in &block.statements {
            if let Statement::Assign { local, .. } = statement {
                definitions[*local] += 1;
            }
            scheduled::for_each_statement_root(statement, |root| {
                count_local_accesses(ir, root, &mut reads, &mut definitions)
            });
        }
        for value in block.terminator.operands() {
            count_local_accesses(ir, ValueUse::Reference(value), &mut reads, &mut definitions);
        }
    }
    for block in ir.blocks.ids() {
        let mut index = 0;
        while index + 1 < raw.ir.blocks[block].statements.len() {
            let ir = &mut raw.ir;
            let pair = (
                &ir.blocks[block].statements[index],
                &ir.blocks[block].statements[index + 1],
            );
            let (
                Statement::Assign {
                    local: source,
                    value,
                },
                Statement::Assign {
                    local: destination,
                    value: copied,
                },
            ) = pair
            else {
                index += 1;
                continue;
            };
            let (source, destination, usage) = (*source, *destination, *value);
            let ValueUse::Definition(value) = usage else {
                index += 1;
                continue;
            };
            if *copied != ValueUse::Reference(value)
                || source == destination
                || ir.values[value].local != Some(source)
                || definitions[source] != 1
                || reads[source] != 1
                || ir.locals[source].parameter.is_some()
                || ir.locals[source].is_this
                || ir.locals[destination].is_this
                || !(ir.locals[source].ty == ir.locals[destination].ty
                    // The producer already supplies the narrow value; its
                    // single copy may commit directly to a wider destination.
                    // Requiring the producer's type preserves any conversion
                    // originally performed by the source binding itself.
                    || ir.values[value].ty == ir.locals[source].ty
                        && super::type_facts::widening(
                            &ir.locals[source].ty,
                            &ir.locals[destination].ty,
                        )
                    || ir.locals[source].ty.is_reference()
                        && ir.locals[destination].ty.is_reference()
                        && reference_assignable.is_some_and(|assignable| {
                            assignable(&ir.locals[source].ty, &ir.locals[destination].ty)
                        }))
            {
                index += 1;
                continue;
            }
            ir.blocks[block].statements[index] = Statement::Assign {
                local: destination,
                value: usage,
            };
            ir.values[value].local = Some(destination);
            raw.remove_statement(block, index + 1);
            reads[source] -= 1;
            definitions[source] -= 1;
        }
    }
}

fn count_local_accesses(
    ir: &MethodIr,
    root: ValueUse,
    reads: &mut IdVec<LocalId, usize>,
    definitions: &mut IdVec<LocalId, usize>,
) {
    let mut work = vec![root];
    while let Some(usage) = work.pop() {
        if let Some(local) = usage.local(ir) {
            reads[local] += 1;
            continue;
        }
        if let Some((local, _)) = ir.values[usage.value()].kind.local_write() {
            definitions[local] += 1;
        }
        usage.for_each_child(ir, |child| work.push(child));
    }
}

/// Successive throw points often observe exactly the same SSA local definition.
/// One capture remains valid until that source changes. Keep the full phi edges
/// for semantic/type analysis, but remove redundant source assignments *before*
/// scheduling so nonexistent snapshot barriers do not force expression temporaries.
fn snapshot_copies(raw: &RawIr, live: &IdVec<ValueId, bool>) -> SnapshotCopies {
    let mut points = BTreeMap::<(BlockId, u32), Vec<(ValueId, ValueId)>>::new();
    for (id, value) in raw.ir.values.iter_enumerated() {
        if !live[id] {
            continue;
        }
        if let ValueKind::Phi { inputs, slot } = &value.kind {
            for input in inputs {
                if let Some(pc) = input.throw_pc
                    && input.value != id
                    && !(slot.is_none() && exception_stack_input(&raw.ir, value.block, input))
                {
                    let copies = points.entry((input.block, pc)).or_default();
                    if !copies.contains(&(id, input.value)) {
                        copies.push((id, input.value));
                    }
                }
            }
        }
    }
    let mut result = HashMap::new();
    let mut current_block = None;
    let mut captured = HashMap::<ValueId, ValueId>::new();
    for (point @ (block, _), copies) in points {
        if Some(block) != current_block {
            captured.clear();
            current_block = Some(block);
        }
        let mut retained = vec![];
        for (destination, source) in copies {
            if captured.get(&destination) != Some(&source) {
                captured.insert(destination, source);
                retained.push((destination, source));
            }
        }
        if retained.is_empty() {
            continue;
        }
        // A handler may protect itself. A destination then also serves as a
        // source local; writing it invalidates any cached capture of its old value.
        captured.retain(|_, source| {
            !retained
                .iter()
                .any(|(destination, _)| source == destination)
        });
        result.insert(point, retained);
    }
    result
}

fn exception_stack_input(ir: &MethodIr, target: BlockId, input: &PhiInput) -> bool {
    input.throw_pc.is_some()
        && matches!(ir.values[input.value].kind, ValueKind::CaughtException { handler } if handler == target)
}

fn contains_inline_effect(
    ir: &MethodIr,
    value: ValueId,
    materialize: &IdVec<ValueId, bool>,
) -> bool {
    let mut pending = vec![(value, true)];
    while let Some((value, root)) = pending.pop() {
        if !root && materialize[value] {
            continue;
        }
        let kind = &ir.values[value].kind;
        if matches!(
            kind,
            ValueKind::Phi { .. } | ValueKind::Parameter { .. } | ValueKind::Allocation { .. }
        ) {
            continue;
        }
        if ir.operation_is_observable(value) {
            return true;
        }
        kind.for_each_operand(|operand| pending.push((operand, false)));
    }
    false
}

/// Phi copies happen on the normal edge after monitor acquisition/release.
/// Keep that boundary explicit: a failed monitorenter must not execute them,
/// and the region builder must not pull them in front of the monitor operation.
fn split_monitor_copies(raw: &mut RawIr) {
    let count = raw.ir.blocks.len();
    for block in raw.ir.blocks.ids().take(count) {
        let ir = &raw.ir;
        if !matches!(ir.blocks[block].terminator, Terminator::Goto(_)) {
            continue;
        }
        let Some(monitor) = ir.blocks[block]
            .statements
            .iter()
            .rposition(|statement| matches!(statement, Statement::Monitor { .. }))
        else {
            continue;
        };
        if monitor + 1 == ir.blocks[block].statements.len() {
            continue;
        }
        let safe = ir.blocks[block].statements[monitor + 1..]
            .iter()
            .all(|statement| {
                let Statement::Assign { value, .. } = statement else {
                    return false;
                };
                !scheduled::value_may_throw(ir, *value)
            });
        if !safe {
            continue;
        }
        raw.split_nonthrowing_suffix(block, monitor + 1);
    }
}

pub(super) fn live_values(raw: &RawIr) -> IdVec<ValueId, bool> {
    let mut live: IdVec<ValueId, _> = vec![false; raw.ir.values.len()].into();
    let mut work = vec![];
    for events in &raw.events {
        for event in events {
            match &event.kind {
                // Recovered choices own their arms' observable operations.
                // Their removed branches no longer root the condition, so
                // retain the whole region even when only a backedge reads its
                // result. Rooting the arm alone would lose its guard and leave
                // no scheduled site at which to evaluate the captured effect.
                EventKind::Define(value) if evaluation::has_effect(raw, *value) => {
                    work.push(*value)
                }
                EventKind::Action(action) => work.extend(action.operands()),
                _ => {}
            }
        }
    }
    for block in &raw.ir.blocks {
        work.extend(block.terminator.operands());
    }
    while let Some(value) = work.pop() {
        if live[value] {
            continue;
        }
        live[value] = true;
        raw.ir.values[value]
            .kind
            .for_each_operand(|operand| work.push(operand));
    }
    live
}

fn statement_expression(kind: &ValueKind) -> bool {
    matches!(
        kind,
        ValueKind::Call { .. }
            | ValueKind::New { .. }
            | ValueKind::Update { .. }
            | ValueKind::CompoundUpdate { .. }
            | ValueKind::InlineAssign { .. }
            | ValueKind::LocalUpdate { .. }
    )
}

/// A shared arm value still needs one local, but its write belongs to the arm.
/// Recovery already proved first-use evaluation reproduces the arm's effects.
/// Process nested choices first and never capture their definitions twice.
fn capture_conditional_values(
    raw: &mut RawIr,
    uses: &IdVec<ValueId, usize>,
) -> std::collections::HashSet<ValueId> {
    let mut regions: Vec<_> = raw
        .conditional_regions
        .iter()
        .map(|(&value, arms)| (value, arms.clone()))
        .collect();
    regions.sort_by_key(|(value, arms)| (arms[0].len() + arms[1].len(), *value));
    let mut removed = std::collections::HashSet::new();
    for (value, arms) in regions {
        let ValueKind::Select {
            then_value,
            else_value,
            ..
        } = raw.ir.values[value].kind
        else {
            unreachable!()
        };
        let mut results = [then_value, else_value];
        for (index, owned) in arms.into_iter().enumerate() {
            let mut pending = owned
                .into_iter()
                .filter(|value| raw.ir.values[*value].local.is_some() && !removed.contains(value))
                .collect();
            results[index] = capture_value(
                &mut raw.ir,
                results[index],
                &mut pending,
                &mut removed,
                uses,
            );
        }
        let ValueKind::Select {
            then_value,
            else_value,
            ..
        } = &mut raw.ir.values[value].kind
        else {
            unreachable!()
        };
        *then_value = results[0];
        *else_value = results[1];
    }
    removed
}

fn capture_constructor_arguments(
    raw: &mut RawIr,
    live: &IdVec<ValueId, bool>,
    uses: &IdVec<ValueId, usize>,
    mut removed: std::collections::HashSet<ValueId>,
    allocation_blocks: &HashMap<u32, BlockId>,
) {
    let original_count = raw.ir.values.len();
    for id in raw.ir.values.ids().take(original_count) {
        let (args, allocation_pc) = match &raw.ir.values[id].kind {
            ValueKind::New {
                args,
                allocation_pc,
                ..
            } => (args, allocation_pc),
            ValueKind::ArrayInitializer {
                values,
                allocation_pc,
                ..
            } => (values, allocation_pc),
            _ => continue,
        };
        let args = args.clone();
        let start = *allocation_pc;
        let end = raw.ir.values[id].pc;
        let block = raw.ir.values[id].block;
        if !super::expressions::same_exception_frame(raw, block, start, end, live) {
            continue;
        }
        let mut dependency = std::collections::HashSet::new();
        let mut work = args.clone();
        while let Some(value) = work.pop() {
            if !dependency.insert(value) {
                continue;
            }
            if !matches!(raw.ir.values[value].kind, ValueKind::Phi { .. }) {
                raw.ir.values[value]
                    .kind
                    .for_each_operand(|operand| work.push(operand));
            }
        }
        let safe_group = raw.events[block]
            .iter()
            .filter(|event| start < event.pc && event.pc < end)
            .all(|event| match &event.kind {
                EventKind::Define(value) => {
                    removed.contains(value)
                        || dependency.contains(value)
                        || !raw.ir.operation_is_observable(*value)
                        || matches!(raw.ir.values[*value].kind, ValueKind::Allocation { .. })
                }
                EventKind::Action(_) => false,
                EventKind::Allocate(value) | EventKind::ReadTarget(value) => {
                    dependency.contains(value)
                }
                EventKind::ThrowPoint => true,
            });
        if !safe_group {
            continue;
        }
        let mut pending: std::collections::HashSet<_> = dependency
            .into_iter()
            .filter(|&value| {
                let source = &raw.ir.values[value];
                source.block == block
                    && start < source.pc
                    && source.pc < end
                    && raw.ir.values[value].local.is_some()
                    && !removed.contains(&value)
                    && !matches!(source.kind, ValueKind::New { allocation_pc, .. }
                        if allocation_blocks.get(&allocation_pc).is_some_and(|&origin| origin != source.block))
                    && !matches!(
                        source.kind,
                        ValueKind::Phi { .. }
                            | ValueKind::CaughtException { .. }
                            | ValueKind::Parameter { .. }
                    )
            })
            .collect();
        if pending.is_empty() {
            continue;
        }
        if !capture_precedes_reads(raw, id, &pending, &removed, live, uses) {
            continue;
        }
        if !captures_dominate_reads(&raw.ir, &args, &pending, |value| {
            raw.ir.values[value].local.is_some()
        }) {
            continue;
        }
        let transformed: Vec<_> = args
            .into_iter()
            .map(|value| capture_value(&mut raw.ir, value, &mut pending, &mut removed, uses))
            .collect();
        match &mut raw.ir.values[id].kind {
            ValueKind::New { args, .. } => *args = transformed,
            ValueKind::ArrayInitializer { values, .. } => *values = transformed,
            _ => unreachable!(),
        }
    }
    for events in &mut raw.events {
        events.retain(
            |event| !matches!(event.kind, EventKind::Define(value) if removed.contains(&value)),
        );
    }
}

/// Capturing a materialized value relocates its local write to the allocation's
/// actual expression site. Pure computations can read that local before the
/// allocation is evaluated, even when every observable effect remains ordered.
/// Check the scheduled roots and operand order before removing those writes.
fn capture_precedes_reads(
    raw: &RawIr,
    allocation: ValueId,
    pending: &std::collections::HashSet<ValueId>,
    removed: &std::collections::HashSet<ValueId>,
    live: &IdVec<ValueId, bool>,
    uses: &IdVec<ValueId, usize>,
) -> bool {
    fn visit(
        ir: &MethodIr,
        value: ValueId,
        root: bool,
        allocation: ValueId,
        pending: &std::collections::HashSet<ValueId>,
        seen: &mut std::collections::HashSet<(ValueId, bool)>,
    ) -> Option<bool> {
        if !root && ir.values[value].local.is_some() {
            return pending.contains(&value).then_some(false);
        }
        if value == allocation {
            return Some(true);
        }
        if !seen.insert((value, root)) {
            return None;
        }
        for operand in ir.values[value].kind.operands() {
            if let Some(result) = visit(ir, operand, false, allocation, pending, seen) {
                return Some(result);
            }
        }
        None
    }

    let mut seen = std::collections::HashSet::new();
    let block = raw.ir.values[allocation].block;
    for event in &raw.events[block] {
        match &event.kind {
            EventKind::Define(value)
                if live[*value]
                    && !pending.contains(value)
                    && !removed.contains(value)
                    && (raw.ir.values[*value].local.is_some()
                        || uses[*value] == 0 && raw.ir.operation_is_observable(*value)) =>
            {
                if let Some(result) = visit(&raw.ir, *value, true, allocation, pending, &mut seen) {
                    return result;
                }
            }
            EventKind::Action(statement) => {
                for value in statement.operands() {
                    if let Some(result) =
                        visit(&raw.ir, value, false, allocation, pending, &mut seen)
                    {
                        return result;
                    }
                }
            }
            _ => {}
        }
    }
    for value in raw.ir.blocks[block].terminator.operands() {
        if let Some(result) = visit(&raw.ir, value, false, allocation, pending, &mut seen) {
            return result;
        }
    }
    false
}

impl RawIr {
    /// During raw recovery, every definition outside the arm remains owned by
    /// its original event. Only arm-owned definitions can become captures.
    pub(super) fn captures_dominate_reads(
        &self,
        roots: &[ValueId],
        owned: &std::collections::HashSet<ValueId>,
    ) -> bool {
        captures_dominate_reads(&self.ir, roots, owned, |_| true)
    }
}

/// First-use captures follow Java operand order, including alternative paths.
/// A definition first reached in one Select arm is unavailable in its sibling
/// and after the choice unless both paths have defined it. Such a definition
/// needs its enclosing statement scope rather than an expression-local write.
fn captures_dominate_reads(
    ir: &MethodIr,
    roots: &[ValueId],
    pending: &std::collections::HashSet<ValueId>,
    is_reference: impl Fn(ValueId) -> bool,
) -> bool {
    enum Visit {
        Value(ValueId),
        Commit(ValueId),
        Choice(ValueId, ValueId),
        Other(ValueId, std::collections::HashSet<ValueId>),
        Join(std::collections::HashSet<ValueId>),
    }
    let mut work: Vec<_> = roots
        .iter()
        .rev()
        .map(|&value| Visit::Value(value))
        .collect();
    let mut captured = std::collections::HashSet::new();
    let mut available = std::collections::HashSet::new();
    while let Some(visit) = work.pop() {
        match visit {
            Visit::Commit(value) => {
                available.insert(value);
            }
            Visit::Choice(left, right) => {
                work.push(Visit::Other(right, available.clone()));
                work.push(Visit::Value(left));
            }
            Visit::Other(value, before) => {
                work.push(Visit::Join(std::mem::replace(&mut available, before)));
                work.push(Visit::Value(value));
            }
            Visit::Join(other) => available.retain(|value| other.contains(value)),
            Visit::Value(value) => {
                let kind = &ir.values[value].kind;
                if matches!(
                    kind,
                    ValueKind::Literal(_)
                        | ValueKind::Allocation { .. }
                        | ValueKind::Parameter { .. }
                        | ValueKind::CaughtException { .. }
                        | ValueKind::Phi { .. }
                ) {
                    continue;
                }
                if pending.contains(&value) {
                    if !captured.insert(value) {
                        if !available.contains(&value) {
                            return false;
                        }
                        continue;
                    }
                    work.push(Visit::Commit(value));
                } else if is_reference(value) {
                    // Before scheduling, pending describes all arm-owned
                    // definitions. After scheduling, a mapped value is a read.
                    continue;
                }
                if let ValueKind::Select {
                    condition,
                    then_value,
                    else_value,
                } = kind
                {
                    work.push(Visit::Choice(*then_value, *else_value));
                    work.push(Visit::Value(condition.right));
                    work.push(Visit::Value(condition.left));
                } else {
                    work.extend(kind.operands().into_iter().rev().map(Visit::Value));
                }
            }
        }
    }
    true
}

fn capture_value(
    ir: &mut MethodIr,
    value: ValueId,
    pending: &mut std::collections::HashSet<ValueId>,
    removed: &mut std::collections::HashSet<ValueId>,
    uses: &IdVec<ValueId, usize>,
) -> ValueId {
    enum Visit {
        Value(ValueUse),
        Finish {
            value: ValueId,
            capture: bool,
            operands_start: usize,
        },
    }
    // Captures cross materialized expression boundaries, so their dependency
    // depth is not bounded by MAX_EXPRESSION_DEPTH. Enter operands in Java
    // order and rebuild parents after their children without using call frames.
    let mut work = vec![Visit::Value(ValueUse::Reference(value))];
    let mut results = vec![];
    while let Some(visit) = work.pop() {
        let (value, capture, operands_start) = match visit {
            Visit::Value(usage) => {
                let value = usage.value();
                let capture = pending.remove(&value);
                let usage = if capture {
                    ValueUse::Definition(value)
                } else {
                    usage
                };
                if usage.local(ir).is_some() {
                    results.push(value);
                    continue;
                }
                work.push(Visit::Finish {
                    value,
                    capture,
                    operands_start: results.len(),
                });
                let start = work.len();
                usage.for_each_child(ir, |child| work.push(Visit::Value(child)));
                work[start..].reverse();
                continue;
            }
            Visit::Finish {
                value,
                capture,
                operands_start,
            } => (value, capture, operands_start),
        };
        // A leaf keeps its identity unless its own definition is captured.
        // In particular, scheduled children exclude phi predecessor edges even
        // when a dead phi has no local; those edges may contain SSA backedges.
        if !capture && results.len() == operands_start {
            results.push(value);
            continue;
        }
        let mut expression = ir.values[value].clone();
        let mut changed = false;
        let mut index = operands_start;
        expression.kind.map_operands(|operand| {
            let replacement = results[index];
            index += 1;
            changed |= replacement != operand;
            replacement
        });
        debug_assert_eq!(index, results.len());
        results.truncate(operands_start);
        if !capture && !changed {
            results.push(value);
            continue;
        }
        // These are fresh expression definitions; the original identity retains
        // its storage for later references, while the copied RHS evaluates inline.
        expression.local = None;
        let expanded = ir.values.push(expression.clone());
        if !capture {
            results.push(expanded);
            continue;
        }
        removed.insert(value);
        // Materialization may have been required only to anchor evaluation before
        // an allocation or choice. Moving that definition to its proven expression
        // site does not require storage unless another use reads the saved result.
        // Keep shared captures as assignments; their first-use dominance was proved
        // before this rewrite, including both paths of nested choices.
        let local = ir.values[value].local.unwrap();
        if uses.get(value) == Some(&1) && ir.locals[local].ty == expression.ty {
            results.push(expanded);
            continue;
        }
        expression.kind = ValueKind::InlineAssign {
            local,
            value: expanded,
        };
        results.push(ir.values.push(expression));
    }
    debug_assert_eq!(results.len(), 1);
    results[0]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Effect {
    Operation(Operation),
    Action(usize),
    Snapshot(usize),
}

/// Fixed bytecode evaluation constraints for the two materialization refinements.
/// Only the expression frontier changes between them. The borrowed raw graph,
/// live values and snapshot barriers remain fixed until this context is dropped.
struct EvaluationOrder<'a> {
    raw: &'a RawIr,
    live: &'a IdVec<ValueId, bool>,
    uses: &'a IdVec<ValueId, usize>,
    snapshots: &'a SnapshotCopies,
    conditional_values: &'a std::collections::HashSet<ValueId>,
    phi_sources: IdVec<BlockId, Vec<ValueId>>,
    value_users: IdVec<ValueId, Vec<ValueId>>,
    blocks: IdVec<BlockId, BlockOrder>,
}

struct BlockOrder {
    original: Vec<Effect>,
    positions: HashMap<Effect, usize>,
}

impl<'a> EvaluationOrder<'a> {
    fn new(
        raw: &'a RawIr,
        live: &'a IdVec<ValueId, bool>,
        uses: &'a IdVec<ValueId, usize>,
        snapshots: &'a SnapshotCopies,
        conditional_values: &'a std::collections::HashSet<ValueId>,
    ) -> Self {
        let mut phi_sources: IdVec<BlockId, _> = vec![Vec::new(); raw.ir.blocks.len()].into();
        let mut value_users: IdVec<ValueId, _> = vec![Vec::new(); raw.ir.values.len()].into();
        for (id, value) in raw.ir.values.iter_enumerated() {
            if !live[id] {
                continue;
            }
            value
                .kind
                .for_each_operand(|operand| value_users[operand].push(id));
            if let ValueKind::Phi { inputs, .. } = &value.kind {
                for input in inputs {
                    if input.throw_pc.is_none() {
                        phi_sources[input.block].push(input.value);
                    }
                }
            }
        }
        let mut blocks = IdVec::with_capacity(raw.events.len());
        for (block, events) in raw.events.iter_enumerated() {
            let mut original = vec![];
            for (event_index, event) in events.iter().enumerate() {
                match event.kind {
                    EventKind::Define(value)
                    | EventKind::Allocate(value)
                    | EventKind::ReadTarget(value)
                        if !conditional_values.contains(&value)
                            && (live[value]
                                || matches!(
                                    raw.ir.values[value].kind,
                                    ValueKind::Allocation { .. }
                                )) =>
                    {
                        if let Some(operation) = evaluation::original(raw, event) {
                            original.push(Effect::Operation(operation));
                        }
                    }
                    EventKind::Action(_) => original.push(Effect::Action(event_index)),
                    EventKind::ThrowPoint if snapshots.contains_key(&(block, event.pc)) => {
                        original.push(Effect::Snapshot(event_index))
                    }
                    _ => {}
                }
            }
            let positions = original
                .iter()
                .enumerate()
                .map(|(index, &effect)| (effect, index))
                .collect();
            blocks.push(BlockOrder {
                original,
                positions,
            });
        }
        Self {
            raw,
            live,
            uses,
            snapshots,
            conditional_values,
            phi_sources,
            value_users,
            blocks,
        }
    }

    fn enforce(&self, materialize: &mut IdVec<ValueId, bool>) {
        let Self {
            raw,
            live,
            uses,
            snapshots,
            conditional_values,
            phi_sources,
            value_users,
            blocks,
        } = self;
        for (block, events) in raw.events.iter_enumerated() {
            let BlockOrder {
                original,
                positions: order,
            } = &blocks[block];
            loop {
                let mut actual = vec![];
                for (event_index, event) in events.iter().enumerate() {
                    match &event.kind {
                        EventKind::Define(value)
                            if live[*value]
                                && !conditional_values.contains(value)
                                && !matches!(
                                    raw.ir.values[*value].kind,
                                    ValueKind::Allocation { .. }
                                )
                                && (materialize[*value] || uses[*value] == 0) =>
                        {
                            expression_effects(raw, *value, true, materialize, &mut actual);
                        }
                        EventKind::Action(action) => {
                            for value in action.operands() {
                                expression_effects(raw, value, false, materialize, &mut actual);
                            }
                            actual.push(Effect::Action(event_index));
                        }
                        EventKind::ThrowPoint if snapshots.contains_key(&(block, event.pc)) => {
                            actual.push(Effect::Snapshot(event_index))
                        }
                        _ => {}
                    }
                }
                for value in raw.ir.blocks[block].terminator.operands() {
                    expression_effects(raw, value, false, materialize, &mut actual);
                }
                // Phi edge sources that remain inline are evaluated after the last
                // original instruction. Materialized/cross-block sources are stable.
                for &value in &phi_sources[block] {
                    expression_effects(raw, value, false, materialize, &mut actual);
                }
                if actual == *original {
                    break;
                }
                let positions: Vec<_> = actual
                    .iter()
                    .map(|effect| order.get(effect).copied())
                    .collect();
                let mut prefix_max = 0;
                let mut marked = vec![];
                for (index, effect) in actual.iter().enumerate() {
                    let Some(position) = positions[index] else {
                        continue;
                    };
                    if position < prefix_max
                        && let Effect::Operation(Operation::Complete(value)) = effect
                        && !materialize[*value]
                        && raw.ir.values[*value].ty != JvmType::Void
                    {
                        marked.push(*value);
                    }
                    prefix_max = prefix_max.max(position);
                }
                let mut suffix_min = usize::MAX;
                for (index, effect) in actual.iter().enumerate().rev() {
                    let Some(position) = positions[index] else {
                        continue;
                    };
                    if position > suffix_min
                        && let Effect::Operation(Operation::Complete(value)) = effect
                        && !materialize[*value]
                        && raw.ir.values[*value].ty != JvmType::Void
                    {
                        marked.push(*value);
                    }
                    suffix_min = suffix_min.min(position);
                }
                if marked.is_empty() {
                    // An earlier definition used only inside a lazy arm must still
                    // execute unconditionally. Capture the missing definition, not
                    // the enclosing choice whose own boundary is already ordered.
                    // The same rule handles sources of cross-block phi edges.
                    for effect in original {
                        if let Effect::Operation(Operation::Complete(value)) = effect
                            && !actual.contains(effect)
                            && !materialize[*value]
                            && raw.ir.values[*value].ty != JvmType::Void
                        {
                            marked.push(*value);
                        }
                    }
                }
                if marked.is_empty() {
                    break;
                }
                // Pin the enclosing expression that crossed the barrier, keeping
                // its argument evaluations together. Pinning every descendant as
                // well would move constructor arguments ahead of allocation.
                let candidates: std::collections::HashSet<_> = marked.iter().copied().collect();
                let pins = marked.into_iter().filter(|&value| {
                    let mut work = value_users[value].clone();
                    let mut seen = std::collections::HashSet::new();
                    while let Some(user) = work.pop() {
                        if materialize[user] || !seen.insert(user) {
                            continue;
                        }
                        if candidates.contains(&user) {
                            return false;
                        }
                        work.extend(value_users[user].iter().copied());
                    }
                    true
                });
                // Repair the earliest displaced expression, then recompute. An
                // inversion involves both the delayed value and the expression it
                // crossed; pinning both at once leaves needless temporaries after
                // the first repair restores their order. In particular, a previous
                // concatenation used by the next one must be stored, but the next
                // fresh StringBuffer does not need a local of its own.
                if let Some(value) =
                    pins.min_by_key(|value| order[&Effect::Operation(Operation::Complete(*value))])
                {
                    materialize[value] = true;
                }
            }
        }
    }
}

fn expression_effects(
    raw: &RawIr,
    value: ValueId,
    root: bool,
    materialize: &IdVec<ValueId, bool>,
    output: &mut Vec<Effect>,
) {
    evaluation::expression(
        raw,
        value,
        root,
        |value, root| root || !materialize[value],
        |operation| output.push(Effect::Operation(operation)),
    );
}

fn parallel_copies(
    ir: &mut MethodIr,
    block: BlockId,
    pc: u32,
    copies: Vec<(LocalId, ValueId)>,
    source_slots: &super::local_identity::SourceSlots,
) -> Vec<Statement> {
    let mut copies: Vec<_> = copies
        .into_iter()
        .filter(|&(local, value)| ir.values[value].local != Some(local))
        .collect();
    let mut output = vec![];
    while !copies.is_empty() {
        // A destination can be overwritten once no remaining source needs its old value.
        let mut ready = None;
        let preferred = copies
            .iter()
            .enumerate()
            .find_map(|(candidate, &(destination, _))| {
                if copies.iter().enumerate().any(|(other, &(_, value))| {
                    other != candidate && reads_matching(ir, value, |source| source == destination)
                }) {
                    return None;
                }
                ready.get_or_insert(candidate);
                // Capture old stack operands before writing the next version
                // of their JVM slot. This is a preference: a cycle falls back
                // to a dependency-safe copy without an extra temporary.
                (!copies.iter().enumerate().any(|(other, &(_, value))| {
                    other != candidate
                        && reads_matching(ir, value, |source| {
                            source_slots.same(destination, source)
                        })
                }))
                .then_some(candidate)
            });
        if let Some(index) = preferred.or(ready) {
            let (local, value) = copies.remove(index);
            output.push(Statement::Assign {
                local,
                value: ValueUse::Reference(value),
            });
            continue;
        }
        let (_, source) = copies[0];
        let local = ir.locals.next_id();
        ir.locals.push(Local {
            name_hint: None,
            name: format!("v{local}"),
            ty: ir.values[source].ty.clone(),
            parameter: None,
            is_this: false,
            needs_default: false,
        });
        output.push(Statement::Assign {
            local,
            value: ValueUse::Reference(source),
        });
        let value = ir.values.next_id();
        ir.values.push(Value {
            local: None,
            kind: ValueKind::Alias(source),
            ty: ir.values[source].ty.clone(),
            pc,
            block,
        });
        ir.values[value].local = Some(local);
        // Snapshot this complete RHS. Replacing unrelated unmapped expressions
        // would conflate distinct definitions; only this pending copy changes.
        copies[0].1 = value;
    }
    output
}

fn reads_matching(ir: &MethodIr, value: ValueId, matches: impl Fn(LocalId) -> bool) -> bool {
    let mut work = vec![value];
    while let Some(value) = work.pop() {
        if let Some(source) = ir.values[value].local {
            if matches(source) {
                return true;
            }
        } else {
            ir.values[value]
                .kind
                .for_each_operand(|operand| work.push(operand));
        }
    }
    false
}

#[cfg(test)]
#[path = "../../tests/unit/schedule.rs"]
mod tests;
