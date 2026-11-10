//! Expand legacy verifier subroutines by call context, without merging their continuations.
use super::*;
use crate::bytecode::Operand;
use crate::hash::HashMap;
use anyhow::{Context, bail, ensure};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct NormalizedCode {
    pub instructions: Vec<Instruction>,
    pub exception_table: Vec<ExceptionHandler>,
    /// Original byte offset for every normalized instruction, including synthetic jumps.
    pub original_pcs: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Frame {
    entry: BlockId,
    continuation: BlockId,
    return_local: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct State {
    block: BlockId,
    calls: Vec<Frame>,
    strip_return_store: bool,
}

#[derive(Clone, Debug)]
enum Transfer {
    Return,
    Goto(usize),
    Conditional {
        target: usize,
        fallthrough: usize,
    },
    Switch {
        default: usize,
        cases: Vec<(i32, usize)>,
    },
    Fallthrough(usize),
}

struct Expansion {
    state: State,
    transfer: Transfer,
    handlers: Vec<(usize, usize, Option<String>)>,
}

fn append(
    output: &mut Vec<Instruction>,
    original_pcs: &mut Vec<u32>,
    mut instruction: Instruction,
    original_pc: u32,
) {
    instruction.pc = output.len() as u32;
    output.push(instruction);
    original_pcs.push(original_pc);
}

fn intern(state: State, states: &mut Vec<State>, ids: &mut HashMap<State, usize>) -> Result<usize> {
    if let Some(&id) = ids.get(&state) {
        return Ok(id);
    }
    // A class-file method can encode an exponential subroutine expansion. Bound
    // that representation rather than allowing untrusted input to exhaust RAM.
    ensure!(
        states.len() < 1_000_000,
        "legacy subroutine expansion exceeds one million blocks"
    );
    let id = states.len();
    states.push(state.clone());
    ids.insert(state, id);
    Ok(id)
}

/// Lexical membership follows a JSR's continuation, not its called body. A
/// finally clause can abandon its return address with break/continue/throw.
/// Code shared with an enclosing context belongs to that outer context, so a
/// transfer there ends the abandoned call before the next loop iteration.
fn memberships(
    graph: &ControlFlowGraph,
    instructions: &[Instruction],
) -> Result<IdVec<BlockId, Vec<BlockId>>> {
    let mut entries = BTreeSet::from([graph.entry]);
    for instruction in instructions
        .iter()
        .filter(|instruction| instruction.opcode == 0xa8)
    {
        entries.insert(
            graph
                .block_at_pc(
                    instructions,
                    instruction.branch_target().context("jsr without target")?,
                )
                .context("jsr target is not a block")?,
        );
    }
    let mut owners: IdVec<BlockId, _> = vec![Vec::new(); graph.blocks.len()].into();
    let mut seen: IdVec<BlockId, _> = vec![None; graph.blocks.len()].into();
    let mut total = 0usize;
    for entry in entries {
        let mut pending = vec![entry];
        while let Some(node) = pending.pop() {
            if seen[node] == Some(entry) {
                continue;
            }
            seen[node] = Some(entry);
            total += 1;
            ensure!(
                total <= 1_000_000,
                "legacy subroutine membership exceeds one million block contexts"
            );
            owners[node].push(entry);
            let block = &graph.blocks[node];
            let last = &instructions[block.end - 1];
            if last.opcode == 0xa8 {
                if block.end < instructions.len() {
                    pending.push(graph.instruction_blocks[block.end]);
                }
            } else if last.opcode != 0xa9 {
                pending.extend(block.successors.iter().copied());
            }
            pending.extend(block.handlers.iter().map(|handler| handler.target));
        }
    }
    Ok(owners)
}

/// Inline `jsr`/`ret` finally subroutines, preserving the calling continuation and
/// ordered exception regions for every clone. Common javac/KJava return-address
/// stores and nested subroutines are supported. Return-address arithmetic or
/// shuffling is rejected explicitly rather than guessed.
pub fn normalize_subroutines(
    instructions: &[Instruction],
    exceptions: &[ExceptionHandler],
) -> Result<NormalizedCode> {
    if !instructions.iter().any(|i| matches!(i.opcode, 0xa8 | 0xa9)) {
        return Ok(NormalizedCode {
            instructions: instructions.to_vec(),
            exception_table: exceptions.to_vec(),
            original_pcs: instructions.iter().map(|i| i.pc).collect(),
        });
    }
    let graph = build(instructions, exceptions)?;
    let owners = memberships(&graph, instructions)?;
    let block_at = |pc| {
        graph
            .block_at_pc(instructions, pc)
            .with_context(|| format!("subroutine target {pc} is not a block"))
    };
    let mut states = Vec::new();
    let mut ids = HashMap::default();
    intern(
        State {
            block: graph.entry,
            calls: Vec::new(),
            strip_return_store: false,
        },
        &mut states,
        &mut ids,
    )?;
    let mut expansions = Vec::new();
    let mut cursor = 0;
    while cursor < states.len() {
        let state = states[cursor].clone();
        cursor += 1;
        let block = &graph.blocks[state.block];
        let last = &instructions[block.end - 1];
        let mut next = |target: BlockId, mut calls: Vec<Frame>, strip_return_store: bool| {
            if !strip_return_store {
                if owners[target].contains(&graph.entry) {
                    calls.clear();
                } else if let Some(index) = calls
                    .iter()
                    .position(|frame| owners[target].contains(&frame.entry))
                {
                    calls.truncate(index + 1);
                }
            }
            intern(
                State {
                    block: target,
                    calls,
                    strip_return_store,
                },
                &mut states,
                &mut ids,
            )
        };
        let transfer = match last.opcode {
            0xa8 => {
                let entry = block_at(last.branch_target().context("jsr without target")?)?;
                ensure!(
                    !state.calls.iter().any(|frame| frame.entry == entry),
                    "recursive jsr at byte {}",
                    last.pc
                );
                ensure!(
                    block.end < instructions.len(),
                    "jsr has no continuation at byte {}",
                    last.pc
                );
                let continuation = graph.instruction_blocks[block.end];
                let entry_block = &graph.blocks[entry];
                let first = instructions[entry_block.start..entry_block.end]
                    .iter()
                    .find(|i| i.opcode != 0x00)
                    .context("empty jsr entry")?;
                let return_local = match (&first.operand, first.opcode) {
                    (Operand::Local(local), 0x3a) => Some(*local),
                    (Operand::None, 0x57) => None,
                    _ => bail!(
                        "unsupported return-address manipulation at jsr entry {}: expected astore or pop",
                        first.pc
                    ),
                };
                let mut calls = state.calls.clone();
                calls.push(Frame {
                    entry,
                    continuation,
                    return_local,
                });
                Transfer::Goto(next(entry, calls, true)?)
            }
            0xa9 => {
                let Operand::Local(local) = last.operand else {
                    bail!("ret without local at byte {}", last.pc)
                };
                let frame = state.calls.last().context("ret without active jsr")?;
                ensure!(
                    frame.return_local == Some(local),
                    "ret at byte {} reads a different return-address local",
                    last.pc
                );
                let continuation = frame.continuation;
                let mut calls = state.calls.clone();
                calls.pop();
                Transfer::Goto(next(continuation, calls, false)?)
            }
            0xa7 => Transfer::Goto(next(
                block_at(last.branch_target().context("goto without target")?)?,
                state.calls.clone(),
                false,
            )?),
            0xaa | 0xab => {
                let Operand::Switch { default, cases } = &last.operand else {
                    bail!("switch without targets")
                };
                let default = next(block_at(*default)?, state.calls.clone(), false)?;
                let cases = cases
                    .iter()
                    .map(|(key, pc)| Ok((*key, next(block_at(*pc)?, state.calls.clone(), false)?)))
                    .collect::<Result<_>>()?;
                Transfer::Switch { default, cases }
            }
            _ if last.is_conditional() => {
                ensure!(
                    block.end < instructions.len(),
                    "conditional at end of subroutine"
                );
                let fallthrough = next(
                    graph.instruction_blocks[block.end],
                    state.calls.clone(),
                    false,
                )?;
                let target = next(
                    block_at(last.branch_target().context("conditional without target")?)?,
                    state.calls.clone(),
                    false,
                )?;
                Transfer::Conditional {
                    target,
                    fallthrough,
                }
            }
            _ if last.can_fall_through() => {
                ensure!(block.end < instructions.len(), "subroutine falls off code");
                Transfer::Fallthrough(next(
                    graph.instruction_blocks[block.end],
                    state.calls.clone(),
                    false,
                )?)
            }
            _ => Transfer::Return,
        };
        let handlers = block
            .handlers
            .iter()
            .map(|edge| {
                Ok((
                    edge.table_index,
                    next(edge.target, state.calls.clone(), false)?,
                    edge.catch_type.clone(),
                ))
            })
            .collect::<Result<_>>()?;
        expansions.push(Expansion {
            state,
            transfer,
            handlers,
        });
    }
    // First assemble with clone IDs as branch placeholders. Every clone has a
    // unique contiguous instruction interval, so exception coverage is exact.
    let mut output = Vec::new();
    let mut original_pcs = Vec::new();
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    for (id, expansion) in expansions.iter().enumerate() {
        let block = &graph.blocks[expansion.state.block];
        starts.push(output.len() as u32);
        let mut skip_store = expansion.state.strip_return_store;
        for (offset, instruction) in instructions[block.start..block.end].iter().enumerate() {
            if skip_store {
                if instruction.opcode == 0 {
                    continue;
                }
                ensure!(
                    matches!(instruction.opcode, 0x3a | 0x57),
                    "subroutine entry changed before return-address store"
                );
                skip_store = false;
                continue;
            }
            let is_last = offset + block.start + 1 == block.end;
            if is_last
                && !matches!(
                    expansion.transfer,
                    Transfer::Fallthrough(_) | Transfer::Return
                )
            {
                break;
            }
            append(
                &mut output,
                &mut original_pcs,
                instruction.clone(),
                instruction.pc,
            );
        }
        let original_last = &instructions[block.end - 1];
        let branch = |target| Instruction {
            pc: 0,
            opcode: 0xa7,
            operand: Operand::Branch(target as u32),
        };
        match &expansion.transfer {
            Transfer::Goto(target) => append(
                &mut output,
                &mut original_pcs,
                branch(*target),
                original_last.pc,
            ),
            Transfer::Conditional {
                target,
                fallthrough,
            } => {
                append(
                    &mut output,
                    &mut original_pcs,
                    Instruction {
                        pc: 0,
                        opcode: original_last.opcode,
                        operand: Operand::Branch(*target as u32),
                    },
                    original_last.pc,
                );
                if *fallthrough != id + 1 {
                    append(
                        &mut output,
                        &mut original_pcs,
                        branch(*fallthrough),
                        original_last.pc,
                    );
                }
            }
            Transfer::Switch { default, cases } => append(
                &mut output,
                &mut original_pcs,
                Instruction {
                    pc: 0,
                    opcode: original_last.opcode,
                    operand: Operand::Switch {
                        default: *default as u32,
                        cases: cases
                            .iter()
                            .map(|&(key, target)| (key, target as u32))
                            .collect(),
                    },
                },
                original_last.pc,
            ),
            Transfer::Fallthrough(target) if *target != id + 1 => append(
                &mut output,
                &mut original_pcs,
                branch(*target),
                original_last.pc,
            ),
            _ => (),
        }
        if output.len() as u32 == starts[id] {
            append(
                &mut output,
                &mut original_pcs,
                Instruction {
                    pc: 0,
                    opcode: 0,
                    operand: Operand::None,
                },
                original_last.pc,
            );
        }
        ends.push(output.len() as u32);
    }
    for instruction in &mut output {
        match &mut instruction.operand {
            Operand::Branch(target) => *target = starts[*target as usize],
            Operand::Switch { default, cases } => {
                *default = starts[*default as usize];
                for (_, target) in cases {
                    *target = starts[*target as usize];
                }
            }
            _ => (),
        }
    }
    let mut ranges = Vec::new();
    for (id, expansion) in expansions.iter().enumerate() {
        for (table_index, target, catch_type) in &expansion.handlers {
            ranges.push((
                *table_index,
                ExceptionHandler {
                    start_pc: starts[id],
                    end_pc: ends[id],
                    handler_pc: starts[*target],
                    catch_type: catch_type.clone(),
                },
            ));
        }
    }
    ranges.sort_by_key(|(table_index, range)| (*table_index, range.start_pc));
    let mut merged: Vec<(usize, ExceptionHandler)> = Vec::new();
    for (table_index, range) in ranges {
        if let Some((previous_index, previous)) = merged.last_mut()
            && *previous_index == table_index
            && previous.end_pc == range.start_pc
            && previous.handler_pc == range.handler_pc
            && previous.catch_type == range.catch_type
        {
            previous.end_pc = range.end_pc;
            continue;
        }
        merged.push((table_index, range));
    }
    Ok(NormalizedCode {
        instructions: output,
        exception_table: merged.into_iter().map(|(_, range)| range).collect(),
        original_pcs,
    })
}
