//! Prepare the scheduled graph shared by every source recovery strategy.
//! Values and Java evaluation order are unchanged.
use crate::ir::{IrBlock, MethodIr, Terminator};

/// A scheduled graph with source switch entries and exception boundaries prepared.
///
/// Keep this immutable while structuring and rendering: structured statements
/// name ranges in this exact graph. Any further lowering must prepare a new
/// graph before creating a new tree.
pub struct PreparedIr(MethodIr);

impl PreparedIr {
    pub fn new(mut ir: MethodIr) -> Self {
        omit_default_switch_entries(&mut ir);
        split_nonthrowing_prefixes(&mut ir);
        Self(ir)
    }

    pub fn ir(&self) -> &MethodIr {
        &self.0
    }
}

fn omit_default_switch_entries(ir: &mut MethodIr) {
    for block in &mut ir.blocks {
        if let Terminator::Switch { cases, default, .. } = &mut block.terminator {
            // Exact shared destinations need no explicit source label. Do this
            // before recovery: ordinary, lexical, dispatcher and cleanup paths
            // all consume this graph. Keep the switch even when no keys remain
            // so its scheduled selector is still evaluated, including effects
            // and exceptions. Raw bytecode/IR retains the original key table.
            cases.retain(|(_, target)| target != default);
        }
    }
}

/// Run after allocation-region lowering and other scheduled-graph preparation,
/// before building the immutable structured tree. Java definite assignment can
/// then see an initialization before the try that consumes it in a catch.
pub fn split_nonthrowing_prefixes(ir: &mut MethodIr) {
    for block in ir.blocks.ids() {
        if ir.blocks[block].handlers.is_empty() {
            continue;
        }
        let prefix = ir.blocks[block]
            .statements
            .iter()
            .take_while(|statement| !super::statement_may_throw(ir, statement))
            .count();
        if prefix == 0 || !super::block_may_throw(ir, block) {
            continue;
        }
        let tail = ir.blocks.next_id();
        let source = &mut ir.blocks[block];
        let suffix = IrBlock {
            caught_exception: None,
            pc: source.pc,
            statements: source.statements.split_off(prefix),
            terminator: std::mem::replace(&mut source.terminator, Terminator::Goto(tail)),
            handlers: std::mem::take(&mut source.handlers),
        };
        ir.blocks.push(suffix);
        // Allocation lowering has consumed its unresolved regions by now. Keep
        // any remaining advisory boundaries indexed correctly for diagnostics.
        for site in &mut ir.initialization_sites {
            if site.block == block && site.statement_start >= prefix {
                site.block = tail;
                site.statement_start -= prefix;
            }
        }
    }
}
