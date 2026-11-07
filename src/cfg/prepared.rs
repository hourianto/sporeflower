//! One normalized instruction stream and its provenance for method analyses.
//! Cheap bytecode-only scans continue to use `bytecode::decode` directly.
use super::{ControlFlowGraph, normalize_subroutines};
use crate::Result;
use crate::bytecode::{Instruction, decode};
use crate::classfile::{Code, ExceptionHandler};
use std::borrow::Cow;

pub struct PreparedCode<'a> {
    pub instructions: Vec<Instruction>,
    pub exception_table: Cow<'a, [ExceptionHandler]>,
    pub graph: ControlFlowGraph,
    original_pcs: Option<Vec<u32>>,
    debug_ranges: Option<Vec<(u32, u32)>>,
}

impl<'a> PreparedCode<'a> {
    pub fn new(code: &'a Code) -> Result<Self> {
        let decoded = decode(&code.bytes)?;
        let (instructions, exception_table, original_pcs, debug_ranges) = if decoded
            .iter()
            .any(|instruction| matches!(instruction.opcode, 0xa8 | 0xa9))
        {
            let normalized = normalize_subroutines(&decoded, &code.exception_table)?;
            let debug_ranges = if code.local_variables.is_empty() {
                Vec::new()
            } else {
                normalized
                    .original_pcs
                    .iter()
                    .map(|&pc| {
                        let index = decoded
                            .binary_search_by_key(&pc, |instruction| instruction.pc)
                            .expect("normalized instruction has an original bytecode origin");
                        (
                            pc,
                            decoded
                                .get(index + 1)
                                .map_or(code.bytes.len() as u32, |instruction| instruction.pc),
                        )
                    })
                    .collect()
            };
            (
                normalized.instructions,
                Cow::Owned(normalized.exception_table),
                Some(normalized.original_pcs),
                Some(debug_ranges),
            )
        } else {
            (
                decoded,
                Cow::Borrowed(code.exception_table.as_slice()),
                None,
                None,
            )
        };
        let graph = ControlFlowGraph::build(&instructions, &exception_table)?;
        Ok(Self {
            instructions,
            exception_table,
            graph,
            original_pcs,
            debug_ranges,
        })
    }

    /// Original byte offset for an instruction index, including JSR clones and
    /// synthetic jumps. The instruction's own PC remains its normalized identity.
    pub fn original_pc(&self, instruction: usize) -> u32 {
        self.original_pcs
            .as_ref()
            .map_or(self.instructions[instruction].pc, |pcs| pcs[instruction])
    }

    /// Original instruction spans indexed by normalized PC for debug-local
    /// attribution. Ordinary bytecode keeps its own PCs; absent debug metadata
    /// needs no span table even when JSR normalization changed the coordinates.
    pub fn debug_ranges(&self) -> Option<&[(u32, u32)]> {
        self.debug_ranges.as_deref()
    }
}
