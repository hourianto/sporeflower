use super::*;
use crate::bytecode::Operand;
use crate::index::IdVec;
use anyhow::{Context, ensure};

pub fn build(
    instructions: &[Instruction],
    exceptions: &[ExceptionHandler],
) -> Result<ControlFlowGraph> {
    ensure!(
        !instructions.is_empty(),
        "cannot build control flow for empty code"
    );
    ensure!(
        instructions[0].pc == 0 && instructions.windows(2).all(|pair| pair[0].pc < pair[1].pc),
        "instruction PCs are not strictly ordered from zero"
    );
    let last_pc = instructions.last().unwrap().pc;
    let mut pc_indices = vec![usize::MAX; last_pc as usize + 1];
    for (index, instruction) in instructions.iter().enumerate() {
        pc_indices[instruction.pc as usize] = index;
    }
    let index_at = |pc: u32| -> Result<usize> {
        let index = pc_indices.get(pc as usize).copied().unwrap_or(usize::MAX);
        ensure!(
            index != usize::MAX,
            "control-flow target {pc} is not an instruction boundary"
        );
        Ok(index)
    };
    let mut leaders = vec![false; instructions.len()];
    leaders[0] = true;
    for (index, instruction) in instructions.iter().enumerate() {
        match &instruction.operand {
            Operand::Branch(pc) => leaders[index_at(*pc)?] = true,
            Operand::Switch { default, cases } => {
                leaders[index_at(*default)?] = true;
                for (_, pc) in cases {
                    leaders[index_at(*pc)?] = true;
                }
            }
            _ => (),
        }
        // Monitor acquisition/release is a region boundary even without a
        // branch. Keeping its normal continuation separate preserves the lock
        // state of subsequent returns, throws, and exception-table dispatch.
        if (instruction.is_conditional()
            || !instruction.can_fall_through()
            || matches!(instruction.opcode, 0xc2 | 0xc3))
            && index + 1 < instructions.len()
        {
            leaders[index + 1] = true;
        }
    }
    for handler in exceptions {
        ensure!(
            handler.start_pc < handler.end_pc,
            "empty or reversed exception range"
        );
        leaders[index_at(handler.start_pc)?] = true;
        leaders[index_at(handler.handler_pc)?] = true;
        if handler.end_pc <= last_pc {
            leaders[index_at(handler.end_pc)?] = true;
        }
    }
    let starts: Vec<usize> = leaders
        .iter()
        .enumerate()
        .filter_map(|(i, leader)| leader.then_some(i))
        .collect();
    let mut instruction_blocks = vec![BlockId::from_index(0); instructions.len()];
    let mut blocks = IdVec::with_capacity(starts.len());
    for (id, &start) in starts.iter().enumerate() {
        let end = starts.get(id + 1).copied().unwrap_or(instructions.len());
        let id = BlockId::from_index(id);
        instruction_blocks[start..end].fill(id);
        blocks.push(BasicBlock {
            id,
            start,
            end,
            successors: Vec::new(),
            predecessors: Vec::new(),
            handlers: Vec::new(),
        });
    }
    let block_at = |pc: u32| -> Result<BlockId> { Ok(instruction_blocks[index_at(pc)?]) };
    for block in &mut blocks {
        let last = &instructions[block.end - 1];
        let mut add = |target: BlockId| {
            if !block.successors.contains(&target) {
                block.successors.push(target);
            }
        };
        match &last.operand {
            Operand::Branch(pc) => add(block_at(*pc)?),
            Operand::Switch { default, cases } => {
                add(block_at(*default)?);
                for (_, pc) in cases {
                    add(block_at(*pc)?);
                }
            }
            _ => (),
        }
        if last.can_fall_through() && block.end < instructions.len() {
            add(instruction_blocks[block.end]);
        }
        let start_pc = instructions[block.start].pc;
        for (table_index, handler) in exceptions.iter().enumerate() {
            if handler.start_pc <= start_pc && start_pc < handler.end_pc {
                block.handlers.push(HandlerEdge {
                    target: block_at(handler.handler_pc).context("exception handler")?,
                    catch_type: handler.catch_type.clone(),
                    table_index,
                });
            }
        }
    }
    for id in blocks.ids() {
        for successor in blocks[id].successors.clone() {
            blocks[successor].predecessors.push(id);
        }
    }
    // A dead padding instruction may sit after the final return. Only reachable
    // fallthrough is malformed; rejecting dead padding breaks old obfuscators.
    let mut reached: IdVec<BlockId, _> = vec![false; blocks.len()].into();
    let mut pending = vec![BlockId::from_index(0)];
    while let Some(id) = pending.pop() {
        if std::mem::replace(&mut reached[id], true) {
            continue;
        }
        let block = &blocks[id];
        let last = &instructions[block.end - 1];
        ensure!(
            !(block.end == instructions.len() && last.can_fall_through()),
            "method falls off end of code at byte {}",
            last.pc
        );
        pending.extend(block.successors.iter().copied());
        pending.extend(super::dispatch::effective(&block.handlers).map(|edge| edge.target));
    }
    Ok(ControlFlowGraph {
        blocks,
        entry: BlockId::from_index(0),
        instruction_blocks,
    })
}
