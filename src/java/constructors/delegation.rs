//! A JVM constructor may delegate and then assign a final field. Java requires
//! that assignment in the constructor which invokes super. Inline its target
//! graph; retain argument evaluation before its super and connect
//! each normal return to the caller's continuation. Throw exits stay abrupt.
use super::*;
use crate::ir::fragments::Rebinder;
use std::collections::HashMap;

pub(super) fn lower(program: &Program<'_>, member: MemberId, ir: &mut MethodIr) -> Result<()> {
    let class = program.class(member.class);
    let assigns_final = ir
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .any(|statement| {
            let Statement::PutField {
                field,
                receiver: Some(receiver),
                ..
            } = statement
            else {
                return false;
            };
            is_this(ir, *receiver, &mut HashSet::new())
                && program
                    .resolve_field(&field.owner, &field.name, &field.descriptor)
                    .is_some_and(|field| {
                        field.class == member.class
                            && program.field(field).access_flags & 0x0018 == 0x0010
                            && program.initialization_plan().permits_final(field)
                    })
        });
    if !assigns_final {
        return Ok(());
    }
    let mut expanded = HashSet::new();
    loop {
        let call = ir.blocks.iter_enumerated().find_map(|(block, body)| {
            body.statements
                .iter()
                .enumerate()
                .find_map(|(statement, action)| {
                    let Statement::ConstructorCall {
                        method,
                        receiver,
                        args,
                        ..
                    } = action
                    else {
                        return None;
                    };
                    (method.owner == class.name).then(|| Delegation {
                        block,
                        statement,
                        target: method.clone(),
                        receiver: *receiver,
                        args: args.clone(),
                    })
                })
        });
        let Some(call) = call else { return Ok(()) };
        let target = program
            .declared_method(member.class, "<init>", &call.target.descriptor)
            .context("final-field constructor delegates to a missing constructor")?;
        ensure!(
            expanded.insert(target),
            "cyclic constructor delegation cannot initialize final fields"
        );
        let target_ir =
            program.decompile_method(target, program.initialization_plan().sites(target))?;
        let Some(blocks) = fragment(&target_ir) else {
            return Ok(());
        };
        if !ir.blocks[call.block].handlers.is_empty() {
            return Ok(());
        }
        let body: Vec<_> = blocks
            .iter()
            .flat_map(|&block| &target_ir.blocks[block].statements)
            .collect();
        // Two original writes to the same final need a different projection.
        // Inlining is sufficient only when delegation and caller initialize
        // disjoint final fields; generated VM-default stores are not present.
        if !final_writes(program, member.class, ir).is_disjoint(&final_writes(
            program,
            member.class,
            &target_ir,
        )) {
            return Ok(());
        }
        let signature = MethodDescriptor::parse(&call.target.descriptor)?;
        let mut replacement = Vec::new();
        let used: HashSet<_> = body
            .iter()
            .flat_map(|statement| {
                statement_reads(&target_ir, statement)
                    .into_iter()
                    .filter_map(|local| target_ir.locals[local].parameter)
            })
            .chain(
                blocks
                    .iter()
                    .flat_map(|&block| target_ir.blocks[block].terminator.operands())
                    .flat_map(|value| value_reads(&target_ir, ValueUse::Reference(value)))
                    .filter_map(|local| target_ir.locals[local].parameter),
            )
            .collect();
        let mut parameters = HashMap::new();
        for (index, (&argument, ty)) in call.args.iter().zip(&signature.parameters).enumerate() {
            if used.contains(&index) {
                let local = ir.locals.next_id();
                ir.locals.push(Local {
                    name_hint: None,
                    name: format!("$delegateArg{local}"),
                    ty: ty.clone(),
                    parameter: None,
                    is_this: false,
                    needs_default: false,
                });
                let reference = add_value(
                    ir,
                    ValueKind::Phi {
                        inputs: vec![],
                        slot: None,
                    },
                    ty.clone(),
                    call.block,
                    Some(local),
                );
                replacement.push(Statement::Assign {
                    local,
                    value: crate::ir::ValueUse::Reference(argument),
                });
                parameters.insert(index, reference);
            } else if ir.values[argument].local.is_none()
                && !pure_value(ir, argument, false, &mut HashSet::new())
            {
                // Even an ignored argument can throw or mutate state. The
                // existing preinit helper transports this before super().
                replacement.push(Statement::Eval(argument));
            }
        }
        let mut copier = Rebinder::new(&target_ir, call.block);
        let locals: BTreeSet<_> = body
            .iter()
            .flat_map(|statement| {
                statement_reads(&target_ir, statement).into_iter().chain(
                    super::super::local_flow::statement_assignments(&target_ir, statement),
                )
            })
            .chain(
                blocks
                    .iter()
                    .flat_map(|&block| target_ir.blocks[block].terminator.operands())
                    .flat_map(|value| value_reads(&target_ir, ValueUse::Reference(value))),
            )
            .chain(
                blocks
                    .iter()
                    .filter_map(|&block| target_ir.blocks[block].caught_exception)
                    .filter_map(|value| target_ir.values[value].local),
            )
            .collect();
        for source in locals {
            let original = &target_ir.locals[source];
            let existing = if original.is_this {
                Some(call.receiver)
            } else {
                original
                    .parameter
                    .and_then(|index| parameters.get(&index).copied())
            };
            if let Some(reference) = existing {
                copier.bind_local(
                    source,
                    ir.values[reference]
                        .local
                        .context("captured constructor local has no storage")?,
                    reference,
                );
            } else {
                let name = format!("$delegate{}", ir.locals.len());
                copier.declare(
                    ir,
                    source,
                    Local {
                        name_hint: None,
                        name,
                        ty: original.ty.clone(),
                        parameter: None,
                        is_this: false,
                        needs_default: original.needs_default,
                    },
                    ValueKind::Phi {
                        inputs: vec![],
                        slot: None,
                    },
                );
            }
        }
        for &block in &blocks {
            let destination = ir.blocks.push(target_ir.blocks[block].clone());
            copier.blocks.insert(block, destination);
        }
        for &block in &blocks {
            if let Some(caught) = target_ir.blocks[block].caught_exception {
                copier.bind_catch(ir, caught)?;
            }
        }
        let mut continuation = ir.blocks[call.block].clone();
        continuation.statements = continuation.statements.split_off(call.statement + 1);
        let continuation = ir.blocks.push(continuation);
        // Phi predecessors describe the caller's old outgoing edges. Those
        // edges now leave its continuation after the copied constructor returns.
        for value in &mut ir.values {
            if let ValueKind::Phi { inputs, .. } = &mut value.kind {
                for input in inputs {
                    if input.block == call.block {
                        input.block = continuation;
                    }
                }
            }
        }
        for &block in &blocks {
            let mut copied = copier.block(ir, block, &target_ir.blocks[block])?;
            if matches!(copied.terminator, Terminator::Return(None)) {
                copied.terminator = Terminator::Goto(continuation);
            }
            ir.blocks[copier.blocks[&block]] = copied;
        }
        ir.blocks[call.block].statements.truncate(call.statement);
        ir.blocks[call.block].statements.extend(replacement);
        ir.blocks[call.block].terminator = Terminator::Goto(copier.blocks[&target_ir.entry]);
    }
}

fn fragment(ir: &MethodIr) -> Option<BTreeSet<BlockId>> {
    if !ir.initialization_sites.is_empty() {
        return None;
    }
    let blocks = reachable(ir, ir.entry, None);
    let mut calls = 0;
    for &block in &blocks {
        for action in &ir.blocks[block].statements {
            if let Statement::ConstructorCall { receiver, .. } = action {
                if !is_this(ir, *receiver, &mut HashSet::new()) {
                    return None;
                }
                calls += 1;
            }
        }
    }
    (calls == 1).then_some(blocks)
}

fn final_writes(program: &Program<'_>, owner: usize, ir: &MethodIr) -> HashSet<usize> {
    ir.blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| {
            let Statement::PutField {
                field,
                receiver: Some(receiver),
                ..
            } = statement
            else {
                return None;
            };
            let id = program.resolve_field(&field.owner, &field.name, &field.descriptor)?;
            (id.class == owner
                && program.field(id).access_flags & 0x0018 == 0x0010
                && program.initialization_plan().permits_final(id)
                && is_this(ir, *receiver, &mut HashSet::new()))
            .then_some(id.index)
        })
        .collect()
}
