//! Pruned SSA needs only values that can actually be read at a block entry.
//! Exception successors participate before each throwing instruction, so a
//! store later in a protected block cannot erase an earlier handler read.
use crate::Result;
use crate::bytecode::{Instruction, Operand};
use crate::cfg::ControlFlowGraph;
use crate::index::{BlockId, IdVec};
use anyhow::{anyhow, ensure};
use std::collections::{BTreeSet, VecDeque};

pub(super) struct LocalLayout {
    pub slots: Vec<u16>,
    indices: Vec<Option<usize>>,
    max_locals: u16,
}

impl LocalLayout {
    pub fn new(
        instructions: &[Instruction],
        parameter_slots: &[u16],
        max_locals: u16,
    ) -> Result<Self> {
        let mut used: BTreeSet<_> = parameter_slots.iter().copied().collect();
        for instruction in instructions {
            match instruction.operand {
                Operand::Local(slot) | Operand::Increment { local: slot, .. } => {
                    used.insert(slot);
                }
                _ => {}
            }
        }
        let slots: Vec<_> = used.into_iter().collect();
        let mut indices = vec![None; max_locals as usize];
        for (index, &slot) in slots.iter().enumerate() {
            ensure!(
                slot < max_locals,
                "local {slot} exceeds max_locals {max_locals}"
            );
            indices[slot as usize] = Some(index);
        }
        Ok(Self {
            slots,
            indices,
            max_locals,
        })
    }

    pub fn index(&self, slot: u16) -> Result<usize> {
        self.optional(slot)
            .ok_or_else(|| anyhow!("invalid local slot {slot}"))
    }

    pub fn optional(&self, slot: u16) -> Option<usize> {
        self.indices.get(slot as usize).copied().flatten()
    }

    pub fn valid_width(&self, slot: u16, width: u16) -> bool {
        slot as u32 + width as u32 <= self.max_locals as u32
    }
}

pub(super) struct LiveLocals {
    entries: IdVec<BlockId, Vec<u64>>,
}

impl LiveLocals {
    pub fn contains(&self, block: BlockId, local: usize) -> bool {
        self.entries[block][local / 64] & (1 << (local % 64)) != 0
    }
}

pub(super) fn analyze(
    instructions: &[Instruction],
    graph: &ControlFlowGraph,
    layout: &LocalLayout,
) -> LiveLocals {
    let words = layout.slots.len().div_ceil(64);
    let mut entries: IdVec<BlockId, _> = vec![vec![0u64; words]; graph.blocks.len()].into();
    let mut predecessors: IdVec<BlockId, _> = graph
        .blocks
        .iter()
        .map(|block| block.predecessors.clone())
        .collect();
    for block in &graph.blocks {
        for handler in crate::cfg::dispatch::effective(&block.handlers) {
            if !predecessors[handler.target].contains(&block.id) {
                predecessors[handler.target].push(block.id);
            }
        }
    }
    let mut queued: IdVec<BlockId, _> = vec![true; graph.blocks.len()].into();
    let mut queue: VecDeque<_> = (graph.blocks.ids()).rev().collect();
    while let Some(block_id) = queue.pop_front() {
        queued[block_id] = false;
        let block = &graph.blocks[block_id];
        let mut live = vec![0; words];
        let mut exceptional = vec![0; words];
        for &successor in &block.successors {
            union(&mut live, &entries[successor]);
        }
        for handler in crate::cfg::dispatch::effective(&block.handlers) {
            union(&mut exceptional, &entries[handler.target]);
        }
        for instruction in instructions[block.start..block.end].iter().rev() {
            match instruction.opcode {
                0x15..=0x19 => {
                    if let Operand::Local(slot) = instruction.operand {
                        set(&mut live, layout.optional(slot), true);
                    }
                }
                0x36..=0x3a => {
                    if let Operand::Local(slot) = instruction.operand {
                        set(&mut live, layout.optional(slot), false);
                        if matches!(instruction.opcode, 0x37 | 0x39) && slot < u16::MAX {
                            set(&mut live, layout.optional(slot + 1), false);
                        }
                    }
                }
                0x84 => {
                    if let Operand::Increment { local, .. } = instruction.operand {
                        set(&mut live, layout.optional(local), true);
                    }
                }
                _ => {}
            }
            if instruction.can_throw() {
                union(&mut live, &exceptional);
            }
        }
        if live != entries[block_id] {
            entries[block_id] = live;
            for &predecessor in &predecessors[block_id] {
                if !queued[predecessor] {
                    queued[predecessor] = true;
                    queue.push_back(predecessor);
                }
            }
        }
    }
    LiveLocals { entries }
}

fn union(target: &mut [u64], source: &[u64]) {
    for (target, source) in target.iter_mut().zip(source) {
        *target |= source;
    }
}

fn set(bits: &mut [u64], index: Option<usize>, value: bool) {
    if let Some(index) = index {
        let mask = 1 << (index % 64);
        if value {
            bits[index / 64] |= mask;
        } else {
            bits[index / 64] &= !mask;
        }
    }
}
