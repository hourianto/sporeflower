//! A JVM constructor may delegate and then assign a final field. Java requires
//! that assignment in the constructor which invokes super. Inline its target
//! graph; retain argument evaluation before its super and connect
//! each normal return to the caller's continuation. Throw exits stay abrupt.
use super::*;
use crate::hash::HashMap;
use crate::ir::fragments::Rebinder;

/// When every path through a finite graph of pure entry tests reaches the
/// same no-argument super call, Java can delegate once before the tests. No
/// path from a constructor body may enter a guard or another delegation.
pub(super) fn shared_implicit_super(
    ir: &MethodIr,
    superclass: Option<&str>,
    calls: &[Delegation],
) -> bool {
    if calls.len() < 2
        || calls.iter().any(|call| {
            Some(call.target.owner.as_str()) != superclass
                || call.target.descriptor != "()V"
                || call.statement != 0
                || !call.args.is_empty()
                || !ir.blocks[call.block].handlers.is_empty()
        })
    {
        return false;
    }
    let mut complete: HashSet<_> = calls.iter().map(|call| call.block).collect();
    let mut guards = HashSet::default();
    let mut pending = vec![(ir.entry, false)];
    while let Some((block, finish)) = pending.pop() {
        if finish {
            complete.insert(block);
        } else if !complete.contains(&block) {
            let body = &ir.blocks[block];
            if !guards.insert(block)
                || !body.statements.is_empty()
                || !body.handlers.is_empty()
                || !matches!(
                    body.terminator,
                    Terminator::Goto(_) | Terminator::Branch { .. } | Terminator::Switch { .. }
                )
                || !body.terminator.operands().iter().all(|&value| {
                    crate::ir::scheduled::values(ir, ValueUse::Reference(value)).all(|usage| {
                        if let Some(local) = usage.local(ir) {
                            ir.locals[local].parameter.is_some()
                        } else {
                            !matches!(ir.values[usage.value()].kind, ValueKind::Phi { .. })
                                && !ir.operation_is_observable(usage.value())
                        }
                    })
                })
            {
                return false;
            }
            pending.push((block, true));
            pending.extend(
                body.terminator
                    .successors()
                    .into_iter()
                    .map(|next| (next, false)),
            );
        }
    }
    ir.blocks.iter_enumerated().all(|(id, block)| {
        guards.contains(&id)
            || block
                .terminator
                .successors()
                .into_iter()
                .chain(crate::cfg::dispatch::effective(&block.handlers).map(|edge| edge.target))
                .all(|target| !complete.contains(&target))
    })
}

pub(super) fn lower(
    cx: &EmissionContext<'_, '_>,
    member: MemberId,
    ir: &mut MethodIr,
) -> Result<()> {
    let program = cx.program;
    let class = program.class(member.class);
    if !ir
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .any(|statement| final_store(program, member.class, ir, statement).is_some())
    {
        return Ok(());
    }
    let mut expanded = HashSet::default();
    loop {
        let call = Delegation::reachable(ir).find(|call| call.target.owner == class.name);
        let Some(call) = call else { return Ok(()) };
        let target = program
            .declared_method(member.class, "<init>", &call.target.descriptor)
            .context("final-field constructor delegates to a missing constructor")?;
        ensure!(
            expanded.insert(target),
            "cyclic constructor delegation cannot initialize final fields"
        );
        let (target_ir, _) = cx.method_ir(target)?;
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
        let mut parameters = HashMap::default();
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
                let reference = ir.push_value(
                    ValueKind::Phi {
                        inputs: vec![],
                        slot: None,
                    },
                    ty.clone(),
                    call.block,
                    ir.blocks[call.block].pc,
                    Some(local),
                );
                replacement.push(Statement::Assign {
                    local,
                    value: crate::ir::ValueUse::Reference(argument),
                });
                parameters.insert(index, reference);
            } else if ValueUse::Reference(argument).is_observable(ir) {
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
    let blocks = ir.reachable_blocks();
    let mut calls = 0;
    for &block in &blocks {
        for action in &ir.blocks[block].statements {
            if let Statement::ConstructorCall { receiver, .. } = action {
                if !ir.aliases_this(*receiver) {
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
            final_store(program, owner, ir, statement).map(|(id, _, _)| id.index)
        })
        .collect()
}
