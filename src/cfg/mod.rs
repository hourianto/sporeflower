//! Dense control-flow graphs keep normal transfer and ordered exception dispatch separate.
use crate::Result;
use crate::bytecode::Instruction;
use crate::classfile::ExceptionHandler;

pub use crate::index::BlockId;
use crate::index::IdVec;

#[derive(Clone, Debug)]
pub struct ControlFlowGraph {
    pub blocks: IdVec<BlockId, BasicBlock>,
    pub entry: BlockId,
    pub instruction_blocks: Vec<BlockId>,
}

#[derive(Clone, Debug)]
pub struct BasicBlock {
    pub id: BlockId,
    pub start: usize,
    pub end: usize,
    pub successors: Vec<BlockId>,
    pub predecessors: Vec<BlockId>,
    /// Ordered lexical table, including shadowed entries and original identity.
    /// Execution analyses use `dispatch::effective` (or `effective_with`).
    pub handlers: Vec<HandlerEdge>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandlerEdge {
    pub target: BlockId,
    pub catch_type: Option<String>,
    pub table_index: usize,
}

mod dominators;
pub(crate) use dominators::Dominators;
mod build;
pub use build::build;
pub mod dispatch;
mod prepared;
pub use prepared::PreparedCode;
mod subroutines;
pub use subroutines::{NormalizedCode, normalize_subroutines};

impl ControlFlowGraph {
    pub fn build(instructions: &[Instruction], exceptions: &[ExceptionHandler]) -> Result<Self> {
        build(instructions, exceptions)
    }
    pub fn block_at_pc(&self, instructions: &[Instruction], pc: u32) -> Option<BlockId> {
        instructions
            .binary_search_by_key(&pc, |i| i.pc)
            .ok()
            .map(|i| self.instruction_blocks[i])
    }
}
