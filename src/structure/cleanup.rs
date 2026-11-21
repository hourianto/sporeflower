//! Cleanup certificates shared by exception inference and source lowering.
//!
//! Facts name normalized bytecode origins, not a particular lexical occurrence
//! or a prepared IR block. Only accepted ownership proofs may consume a rethrow.
use super::{SourceHandlers, StructuredMethod, regions::RegionGraph};
use crate::{
    index::BlockId,
    ir::{MethodIr, Terminator},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Default)]
pub(crate) struct CleanupFacts {
    rethrows: BTreeMap<u32, BTreeSet<u32>>,
}

impl CleanupFacts {
    pub fn consumes(&self, handler_pc: u32, throw_pc: u32) -> bool {
        self.rethrows
            .get(&handler_pc)
            .is_some_and(|throws| throws.contains(&throw_pc))
    }

    pub(super) fn record(
        &mut self,
        ir: &MethodIr,
        handler: BlockId,
        rethrows: impl IntoIterator<Item = BlockId>,
    ) {
        let origins = self.rethrows.entry(ir.blocks[handler].pc).or_default();
        for block in rethrows {
            if let Terminator::Throw { pc, .. } = ir.blocks[block].terminator {
                origins.insert(pc);
            }
        }
    }

    pub(super) fn extend(&mut self, facts: Self) {
        for (handler, rethrows) in facts.rethrows {
            self.rethrows.entry(handler).or_default().extend(rethrows);
        }
    }
}

pub(super) struct Recovery {
    pub regions: RegionGraph,
    pub suppressed_monitors: Vec<(BlockId, usize)>,
}

impl Recovery {
    pub fn lower(self, source: Option<SourceHandlers<'_>>) -> crate::Result<StructuredMethod> {
        let mut result = self.regions.lower(source)?;
        result.suppressed_monitors = self.suppressed_monitors;
        Ok(result)
    }
}

/// Recover semantic ownership before selecting the outer source strategy or
/// normalizing predicates, continuations, and lexical cleanup presentation.
pub(super) fn recover(
    analysis: &super::analysis::MethodAnalysis<'_>,
    source: Option<SourceHandlers<'_>>,
) -> Recovery {
    let ir = analysis.ir;
    let monitors = ir.blocks.iter().any(|block| {
        block
            .statements
            .iter()
            .any(|statement| matches!(statement, crate::ir::Statement::Monitor { enter: true, .. }))
    });
    let mut monitor_facts = CleanupFacts::default();
    if monitors && let Some(proof) = super::synchronized::validate_monitor_stacks(analysis) {
        monitor_facts = proof.cleanups(ir);
        if let Some(mut recovered) = super::synchronized::recover(analysis, proof, source) {
            recovered.regions.cleanups.extend(monitor_facts);
            return recovered;
        }
    }
    let mut regions = RegionGraph::new(ir);
    regions.cleanups = monitor_facts;
    regions.recover_cleanups(ir, |regions, candidate| {
        super::finally::plan_candidate(analysis, regions, candidate, source)
    });
    Recovery {
        regions,
        suppressed_monitors: Vec::new(),
    }
}

pub(crate) fn facts(
    prepared: &super::PreparedIr,
    method: crate::program::MethodFacts<'_>,
) -> CleanupFacts {
    let analysis = super::analysis::MethodAnalysis::new(prepared.ir(), Some(method));
    recover(&analysis, None).regions.cleanups
}
