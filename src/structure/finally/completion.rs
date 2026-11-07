//! A cleanup's terminal outcome is separate from its duplicated operations.
//! A returning handler suppresses its pending exception; normal copies retain
//! their own result. Returns may cross cleanup only with a pure, stable value.
use super::equivalent;
use crate::ir::{MethodIr, Terminator};
use crate::structure::{BlockId, Catch, FlowEnd, FlowGraph, Stmt};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy)]
pub(super) enum Completion {
    ResumeCaught,
    Return(BlockId),
}

impl Completion {
    pub fn certify(
        ir: &MethodIr,
        caught: crate::ir::ValueId,
        ends: impl Iterator<Item = BlockId>,
        cleanup: &[Stmt],
    ) -> Option<Self> {
        let mut result = None;
        for block in ends {
            let next = match ir.blocks[block].terminator {
                Terminator::Return(value)
                    if value.is_none_or(|value| equivalent::stable_return(ir, value, cleanup)) =>
                {
                    Self::Return(block)
                }
                Terminator::Throw { value, .. } if value == caught => Self::ResumeCaught,
                _ => return None,
            };
            if let Some(previous) = result {
                match (previous, next) {
                    (Self::ResumeCaught, Self::ResumeCaught) => (),
                    (Self::Return(left), Self::Return(right))
                        if equivalent::same_exit(ir, &Stmt::Return(left), &Stmt::Return(right)) => {
                    }
                    _ => return None,
                }
            }
            result = Some(next);
        }
        result
    }

    pub fn returns(self) -> bool {
        matches!(self, Self::Return(_))
    }

    pub fn protect(self, body: Vec<Stmt>, handler: BlockId) -> Vec<Stmt> {
        match self {
            Self::ResumeCaught => body,
            // This return replaces the pending exception before the finally
            // executes. A cleanup exception still replaces the pending return.
            Self::Return(block) => vec![Stmt::Try {
                body,
                catches: vec![Catch {
                    handler,
                    catch_type: None,
                    body: vec![Stmt::Return(block)],
                }],
            }],
        }
    }
}

/// Only empty control connectors may separate a completed copy from its
/// return. Do not move assignments, reads with effects, or a branch decision.
pub(super) fn returning_copy(
    ir: &MethodIr,
    graph: &FlowGraph,
    replacements: &BTreeMap<BlockId, Vec<Stmt>>,
    mut block: BlockId,
    mut index: usize,
    canonical: &[Stmt],
    copied: &[Stmt],
) -> Option<BlockId> {
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(block)
            || replacements.contains_key(&block)
            || index != ir.blocks[block].statements.len()
        {
            return None;
        }
        match (&ir.blocks[block].terminator, &graph.blocks[block].end) {
            (Terminator::Return(value), FlowEnd::Return) => {
                return value
                    .is_none_or(|value| {
                        equivalent::stable_return(ir, value, canonical)
                            && equivalent::stable_return(ir, value, copied)
                    })
                    .then_some(block);
            }
            (_, FlowEnd::Goto(next)) => {
                block = *next;
                index = 0;
            }
            _ => return None,
        }
    }
}
