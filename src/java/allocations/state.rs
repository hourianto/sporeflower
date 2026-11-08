//! Typed transport for allocation arguments and partially assigned locals.
//!
//! The state object is allocated inside the target NEW's first argument. It is
//! visible to the caller before the helper starts, so an exception halfway
//! through argument evaluation exposes exactly the completed local assignments.
use super::*;
use crate::ir::scheduled_liveness::LocalLiveness;
use crate::ir::{CompareOp, Condition, Literal, ValueUse};
use crate::java::generated::{Carrier, Slot};

pub(super) fn lower(
    context: &mut LoweringContext<'_, '_>,
    ir: &mut MethodIr,
    mut selection: Selection,
    handler_prefixes: &mut BTreeMap<BlockId, Vec<Statement>>,
) -> Result<AllocationHelper> {
    let site = &selection.site;
    let mut normal_exports = selection.exports.clone();
    if let Some(Statement::Assign { local, value }) =
        ir.blocks[selection.shape.end].statements.get(selection.end)
        && *value == ValueUse::Definition(selection.value)
    {
        // The constructor result commits after the helper's argument writes.
        // Coalescing can reuse that storage for an older exceptional snapshot;
        // restore it on failure, but never overwrite the new result on success.
        normal_exports.remove(local);
    }
    if !selection.exports.is_empty() {
        let root = ir.blocks[selection.shape.end].statements.get(selection.end);
        ensure!(
            matches!(root, Some(Statement::Assign { value: ValueUse::Definition(value), .. } | Statement::Eval(value)) if *value == selection.value),
            "external allocation {} at {} needs live-output transport through a containing expression",
            site.class,
            site.pc
        );
        ensure!(
            !selection
                .exports
                .iter()
                .any(|&local| ir.locals[local].is_this),
            "allocation argument region writes the receiver identity"
        );
    }
    let handlers: BTreeSet<_> = ir.blocks[site.block]
        .handlers
        .iter()
        .map(|handler| handler.target)
        .collect();
    let mut exceptional_exports = BTreeMap::new();
    if !handlers.is_empty() && !selection.exports.is_empty() {
        let liveness = LocalLiveness::analyze(ir)?;
        for handler in handlers {
            let mut live = liveness.at_handler(ir, handler);
            if let Some(prefix) = handler_prefixes.get(&handler) {
                for statement in prefix {
                    live.extend(statement_reads(ir, statement));
                }
            }
            let exports: BTreeSet<_> = selection.exports.intersection(&live).copied().collect();
            selection.inputs.extend(exports.iter().copied());
            if !exports.is_empty() {
                exceptional_exports.insert(handler, exports);
            }
        }
    }
    // A conditional write must preserve the incoming value on the untaken arm.
    selection
        .inputs
        .extend(selection.exports.difference(&selection.definite).copied());
    let carrier = Carrier::new(
        carrier_name(context, site.pc),
        selection.parameters.clone(),
        ir,
        selection.exports.iter().copied(),
    );
    let carrier_type = carrier.ty();
    let name = helper_name(
        context.cx.program,
        context.cx.types,
        context.member,
        site.pc,
    );
    let all_locals = selection
        .inputs
        .union(&selection.written)
        .copied()
        .collect();
    // Build the caller's state operand before extraction borrows the graph.
    // These values execute only when the completed helper call is installed.
    let caller_state_local = ir.locals.next_id();
    ir.locals.push(Local {
        name_hint: None,
        name: format!("$arguments{}_{}", context.member.index, site.pc),
        ty: carrier_type.clone(),
        parameter: None,
        is_this: false,
        needs_default: true,
    });
    let caller_state = append_value(
        ir,
        ValueKind::Phi {
            inputs: vec![],
            slot: None,
        },
        carrier_type.clone(),
        site.block,
        site.pc,
        Some(caller_state_local),
    );
    let allocated_state = append_value(
        ir,
        ValueKind::New {
            class: carrier.name.clone(),
            descriptor: "()V".into(),
            args: vec![],
            allocation_pc: site.pc,
        },
        carrier_type.clone(),
        site.block,
        site.pc,
        None,
    );
    let capture_state = append_value(
        ir,
        ValueKind::InlineAssign {
            local: caller_state_local,
            value: allocated_state,
        },
        carrier_type.clone(),
        site.block,
        site.pc,
        None,
    );
    let mut region = Region::new(
        ir,
        &selection.inputs,
        &all_locals,
        selection.parameters[0].clone(),
    )?;
    region.prepare_shape(&selection.shape);
    let slot = u16::from(region.instance)
        + region
            .captured
            .iter()
            .map(|&local| ir.locals[local].ty.slots())
            .sum::<u16>();
    ensure!(
        slot < 255,
        "allocation state helper exceeds 255 parameter slots"
    );
    let state_local = region.ir.locals.next_id();
    region.ir.locals.push(Local {
        name_hint: None,
        name: "$state".into(),
        ty: carrier_type.clone(),
        parameter: Some(region.captured.len()),
        is_this: false,
        needs_default: false,
    });
    let helper_entry = region.ir.entry;
    let state = append_value(
        &mut region.ir,
        ValueKind::Parameter {
            slot,
            index: Some(region.captured.len()),
        },
        carrier_type.clone(),
        helper_entry,
        site.pc,
        Some(state_local),
    );
    region.ir.parameters.push(state);
    for &local in &selection.exports {
        region
            .copy
            .bind_field(local, carrier.field(Slot::Local(local)), Some(state));
    }

    let mut initializations = Vec::new();
    for &local in selection.exports.intersection(&selection.inputs) {
        initializations.push(carrier.store(
            state,
            Slot::Local(local),
            region.copy.reference(local),
        ));
    }
    region.clone_blocks(&selection.shape)?;
    initializations.append(&mut region.ir.blocks[helper_entry].statements);
    region.ir.blocks[helper_entry].statements = initializations;
    let helper_end = region.copy.blocks[&selection.shape.end];
    for (index, &argument) in selection.arguments.iter().enumerate() {
        let value = region.value(ValueUse::Reference(argument))?;
        region.ir.blocks[helper_end].statements.push(carrier.store(
            state,
            Slot::Argument(index),
            value,
        ));
    }
    let returned = carrier.load(
        &mut region.ir,
        state,
        Slot::Argument(0),
        helper_end,
        site.pc,
    );
    region.ir.blocks[helper_end].terminator = Terminator::Return(Some(returned));

    let mut declarations = CodeWriter::new(1);
    declarations.line("");
    carrier.emit(context.cx.types, &mut declarations);
    let (helper, call) = region.finish(context, name, declarations.text, Some(capture_state))?;
    let call = append_value(
        ir,
        call,
        selection.parameters[0].clone(),
        site.block,
        ir.values[selection.value].pc,
        None,
    );
    let mut arguments = vec![call];
    for index in 1..selection.parameters.len() {
        arguments.push(carrier.load(ir, caller_state, Slot::Argument(index), site.block, site.pc));
    }
    let ValueKind::New { args, .. } = &mut ir.values[selection.value].kind else {
        unreachable!()
    };
    *args = arguments;
    graph::place(ir, &selection.shape, selection.value);
    let null = append_value(
        ir,
        ValueKind::Literal(Literal::Null),
        carrier_type,
        site.block,
        site.pc,
        None,
    );
    let mut normal_restore = restore(
        ir,
        &carrier,
        caller_state,
        null,
        &normal_exports,
        site.block,
        false,
    )?;
    normal_restore.push(Statement::Assign {
        local: caller_state_local,
        value: ValueUse::Reference(null),
    });
    if site.statement_start < ir.blocks[site.block].statements.len() {
        ir.blocks[site.block].statements.splice(
            site.statement_start + 1..site.statement_start + 1,
            normal_restore,
        );
    }
    for (handler, outputs) in exceptional_exports {
        let mut restore = restore(ir, &carrier, caller_state, null, &outputs, handler, true)?;
        restore.push(Statement::Assign {
            local: caller_state_local,
            value: ValueUse::Reference(null),
        });
        let previous = handler_prefixes.entry(handler).or_default();
        // An outer extraction must expose an inner state before that state's
        // previously queued restoration executes.
        restore.append(previous);
        *previous = restore;
    }
    Ok(helper)
}

fn restore(
    ir: &mut MethodIr,
    carrier: &Carrier,
    state: ValueId,
    null: ValueId,
    outputs: &BTreeSet<LocalId>,
    block: BlockId,
    conditional: bool,
) -> Result<Vec<Statement>> {
    let mut statements = Vec::new();
    for &local in outputs {
        let ty = ir.locals[local].ty.clone();
        let field_value = carrier.load(ir, state, Slot::Local(local), block, ir.blocks[block].pc);
        let value = if conditional {
            let fallback =
                local_value(ir, local).context("exceptional output has no prior local identity")?;
            append_value(
                ir,
                ValueKind::Select {
                    condition: Condition {
                        op: CompareOp::Ne,
                        left: state,
                        right: null,
                    },
                    then_value: field_value,
                    else_value: fallback,
                },
                ty,
                block,
                ir.blocks[block].pc,
                None,
            )
        } else {
            field_value
        };
        statements.push(Statement::Assign {
            local,
            value: ValueUse::Reference(value),
        });
    }
    Ok(statements)
}

fn append_value(
    ir: &mut MethodIr,
    kind: ValueKind,
    ty: JvmType,
    block: BlockId,
    pc: u32,
    local: Option<LocalId>,
) -> ValueId {
    let id = ir.values.next_id();
    ir.values.push(Value {
        local,
        kind,
        ty,
        pc,
        block,
    });
    id
}

fn carrier_name(context: &LoweringContext<'_, '_>, pc: u32) -> String {
    super::super::generated::symbol_in(
        context.cx.program,
        context.cx.types,
        context.cx.helper_container.as_deref(),
        &format!("$sf$Arguments_{}_{}", context.member.index, pc),
    )
}
