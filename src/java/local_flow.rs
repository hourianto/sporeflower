//! Read/write facts for scheduled Java locals, including assignments inside expressions.
#[cfg(test)]
use crate::ir::ValueKind;
use crate::ir::scheduled;
use crate::ir::{MethodIr, Statement, ValueUse, local_effects};
use std::collections::BTreeSet;

pub(super) fn value_reads(ir: &MethodIr, usage: ValueUse) -> BTreeSet<crate::ir::LocalId> {
    let mut reads = BTreeSet::new();
    expression_effects(ir, usage, &mut reads, &mut BTreeSet::new());
    reads
}

pub(super) fn statement_reads(
    ir: &MethodIr,
    statement: &Statement,
) -> BTreeSet<crate::ir::LocalId> {
    let mut reads = BTreeSet::new();
    statement_effects(ir, statement, &mut reads, &mut BTreeSet::new());
    reads
}

pub(super) fn expression_effects(
    ir: &MethodIr,
    usage: ValueUse,
    reads: &mut BTreeSet<crate::ir::LocalId>,
    defined: &mut BTreeSet<crate::ir::LocalId>,
) {
    local_effects::expression(ir, usage).reads_before_writes(reads, defined);
}

pub(super) fn value_assignments(ir: &MethodIr, usage: ValueUse) -> BTreeSet<crate::ir::LocalId> {
    inline_assignments(ir, scheduled::values(ir, usage)).collect()
}

pub(super) fn statement_assignments(
    ir: &MethodIr,
    statement: &Statement,
) -> BTreeSet<crate::ir::LocalId> {
    let mut result: BTreeSet<_> =
        inline_assignments(ir, scheduled::statement_values(ir, statement)).collect();
    if let Statement::Assign { local, .. } = statement {
        result.insert(*local);
    }
    result
}

/// The union of reads and writes needs neither effect ordering nor a temporary
/// effect tree. Even a read preceded by a write contributes the same local.
pub(super) fn value_accesses(ir: &MethodIr, usage: ValueUse) -> BTreeSet<crate::ir::LocalId> {
    let mut result = BTreeSet::new();
    collect_accesses(ir, scheduled::values(ir, usage), &mut result);
    result
}

pub(super) fn statement_accesses(
    ir: &MethodIr,
    statement: &Statement,
) -> BTreeSet<crate::ir::LocalId> {
    let mut result = BTreeSet::new();
    collect_statement_accesses(ir, statement, &mut result);
    result
}

fn inline_assignments<'a>(
    ir: &'a MethodIr,
    values: scheduled::Values<'a>,
) -> impl Iterator<Item = crate::ir::LocalId> + 'a {
    values
        .operations()
        .filter_map(|value| ir.values[value].kind.local_write().map(|(local, _)| local))
}

fn collect_accesses(
    ir: &MethodIr,
    values: scheduled::Values<'_>,
    result: &mut BTreeSet<crate::ir::LocalId>,
) {
    for usage in values {
        if let Some(local) = usage.local(ir) {
            result.insert(local);
        } else if let Some((local, _)) = ir.values[usage.value()].kind.local_write() {
            result.insert(local);
        }
    }
}

fn collect_statement_accesses(
    ir: &MethodIr,
    statement: &Statement,
    result: &mut BTreeSet<crate::ir::LocalId>,
) {
    collect_accesses(ir, scheduled::statement_values(ir, statement), result);
    if let Statement::Assign { local, .. } = statement {
        result.insert(*local);
    }
}

pub(super) fn statement_effects(
    ir: &MethodIr,
    statement: &Statement,
    reads: &mut BTreeSet<crate::ir::LocalId>,
    defined: &mut BTreeSet<crate::ir::LocalId>,
) {
    local_effects::statement(ir, statement, false).reads_before_writes(reads, defined);
}

pub(super) fn active_locals(ir: &MethodIr) -> BTreeSet<crate::ir::LocalId> {
    let mut result = BTreeSet::new();
    for block in &ir.blocks {
        for statement in &block.statements {
            collect_statement_accesses(ir, statement, &mut result);
        }
        for value in block.terminator.operands() {
            collect_accesses(
                ir,
                scheduled::values(ir, ValueUse::Reference(value)),
                &mut result,
            );
        }
    }
    result
}

pub(super) fn live_at_entry(ir: &MethodIr) -> crate::Result<BTreeSet<crate::ir::LocalId>> {
    Ok(crate::ir::scheduled_liveness::LocalLiveness::analyze(ir)?.at(ir.entry))
}

#[cfg(test)]
#[path = "../../tests/unit/local_flow.rs"]
mod tests;
