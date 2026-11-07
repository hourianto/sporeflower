use super::frames;
use crate::{
    Result,
    bytecode::{Instruction, Operand},
    cfg::{BlockId, ControlFlowGraph, PreparedCode},
    classfile::{ClassFile, Method},
    index::IdVec,
    program::{MemberId, Program},
};
use std::{
    collections::{BTreeSet, VecDeque},
    sync::Arc,
};

pub(super) struct Summary {
    pub calls: Vec<Call>,
    pub throws: Vec<Throw>,
    pub implicit_cleanups: BTreeSet<BlockId>,
}
pub(super) struct Call {
    pub pc: u32,
    pub target: Option<MemberId>,
    pub dispatch_owner: Option<String>,
    pub reflection_classes: Option<BTreeSet<Arc<str>>>,
    pub handlers: Vec<crate::cfg::HandlerEdge>,
}
pub(super) struct Throw {
    pub pc: u32,
    pub instruction_pc: u32,
    pub value: frames::ThrowValue,
    pub handlers: Vec<crate::cfg::HandlerEdge>,
}

pub(super) fn method(program: &Program<'_>, class: &ClassFile, method: &Method) -> Result<Summary> {
    let code = method.code.as_ref().expect("scanning concrete method");
    let prepared = PreparedCode::new(code)?;
    let instructions = &prepared.instructions;
    let cfg = &prepared.graph;
    let reachable = reachable(program, instructions, cfg);
    let mut frames = if instructions
        .iter()
        .any(|instruction| instruction.opcode == 0xbf || frames::is_reflection(class, instruction))
    {
        frames::analyze(program, class, method, instructions, cfg)?
    } else {
        frames::Summary::default()
    };
    // Share accepted cleanup ownership certificates with source lowering.
    // Lexical recovery and later source choices cannot change these effects.
    let implicit_cleanups = if frames.throws.values().any(|value| {
        value
            .origins
            .iter()
            .any(|origin| matches!(origin, frames::Origin::Caught { .. }))
    }) {
        crate::ir::lift(class, method, instructions, cfg)
            .ok()
            .map(|ir| {
                let prepared = crate::structure::PreparedIr::new(ir);
                let facts = crate::structure::cleanup_facts(&prepared);
                let mut handlers = std::collections::BTreeMap::<BlockId, bool>::new();
                for (&pc, value) in &frames.throws {
                    for origin in &value.origins {
                        if let frames::Origin::Caught { block, .. } = origin {
                            let handler_pc = instructions[cfg.blocks[*block].start].pc;
                            // A handler can feed multiple rethrows or lexical
                            // occurrences. Every original use must be consumed.
                            *handlers.entry(*block).or_insert(true) &=
                                facts.consumes(handler_pc, pc);
                        }
                    }
                }
                handlers
                    .into_iter()
                    .filter_map(|(handler, consumed)| consumed.then_some(handler))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        BTreeSet::new()
    };
    let mut result = Summary {
        calls: Vec::new(),
        throws: Vec::new(),
        implicit_cleanups,
    };
    for block in &cfg.blocks {
        if !reachable[block.id] {
            continue;
        }
        for (offset, instruction) in instructions[block.start..block.end].iter().enumerate() {
            if matches!(instruction.opcode, 0xb6..=0xb9) {
                let index = match instruction.operand {
                    Operand::Constant(index) | Operand::InterfaceCall { index, .. } => index,
                    _ => continue,
                };
                let member = class.constant_pool.member_ref(index)?;
                let target = program.resolve_method(member.owner, member.name, member.descriptor);
                result.calls.push(Call {
                    pc: instruction.pc,
                    target,
                    reflection_classes: frames.reflection.remove(&instruction.pc).flatten(),
                    dispatch_owner: matches!(instruction.opcode, 0xb6 | 0xb9)
                        .then(|| member.owner.to_owned()),
                    handlers: active_handlers(program, &block.handlers).cloned().collect(),
                });
            } else if instruction.opcode == 0xbf
                && let Some(value) = frames.throws.remove(&instruction.pc)
            {
                let pc = prepared.original_pc(block.start + offset);
                // Exact normalized origin survives source block merging and splitting.
                let instruction_pc = instruction.pc;
                result.throws.push(Throw {
                    pc,
                    instruction_pc,
                    value,
                    handlers: active_handlers(program, &block.handlers).cloned().collect(),
                });
            }
        }
    }
    Ok(result)
}

fn reachable(
    program: &Program<'_>,
    instructions: &[Instruction],
    cfg: &ControlFlowGraph,
) -> IdVec<BlockId, bool> {
    let mut reached: IdVec<BlockId, bool> = vec![false; cfg.blocks.len()].into();
    let mut pending = VecDeque::from([cfg.entry]);
    while let Some(block) = pending.pop_front() {
        if std::mem::replace(&mut reached[block], true) {
            continue;
        }
        pending.extend(cfg.blocks[block].successors.iter().copied());
        if instructions[cfg.blocks[block].start..cfg.blocks[block].end]
            .iter()
            .any(Instruction::can_throw)
        {
            pending.extend(
                active_handlers(program, &cfg.blocks[block].handlers).map(|handler| handler.target),
            );
        }
    }
    reached
}

pub(super) fn active_handlers<'a>(
    program: &'a Program<'_>,
    handlers: &'a [crate::cfg::HandlerEdge],
) -> impl Iterator<Item = &'a crate::cfg::HandlerEdge> {
    // Semantic reachability uses hierarchy shadowing on the bytecode table.
    // This differs from source eligibility, which additionally removes checked
    // catches unsupported by the effects of their particular lexical body.
    crate::cfg::dispatch::effective_with(handlers, |current, earlier| {
        program.is_subtype(current, earlier)
    })
}
