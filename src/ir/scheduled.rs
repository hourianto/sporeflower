//! Projection from SSA dependencies to expressions that Java actually evaluates.
//!
//! A reference to a materialized value reads its local; its definition evaluates
//! the original operation. Inline assignments evaluate their RHS definition,
//! while phi inputs describe incoming edges and are never expression children.
//! These walks collect possible uses, not execution traces: both select arms
//! are visited and repeated uses are deduplicated. Ordered effects belong in
//! `local_effects`, which also models alternatives and writes.
pub(crate) use super::ValueUse;
use super::{LocalId, MethodIr, Statement, ValueId, ValueKind};
use crate::hash::HashSet;

impl ValueUse {
    pub fn value(self) -> ValueId {
        match self {
            Self::Reference(value) | Self::Definition(value) => value,
        }
    }

    pub fn map_value(self, map: impl FnOnce(ValueId) -> ValueId) -> Self {
        match self {
            Self::Reference(value) => Self::Reference(map(value)),
            Self::Definition(value) => Self::Definition(map(value)),
        }
    }

    /// Parameters and caught exceptions are bindings even at a definition root.
    pub fn local(self, ir: &MethodIr) -> Option<LocalId> {
        let value = self.value();
        if matches!(self, Self::Reference(_))
            || matches!(
                ir.values[value].kind,
                ValueKind::Parameter { .. } | ValueKind::CaughtException { .. }
            )
        {
            ir.values[value].local
        } else {
            None
        }
    }

    /// Direct evaluated children, in source operand order. Select arms remain
    /// alternatives; callers needing path-sensitive effects must retain that.
    pub fn for_each_child(self, ir: &MethodIr, mut visit: impl FnMut(Self)) {
        if self.local(ir).is_some() {
            return;
        }
        let kind = &ir.values[self.value()].kind;
        if let Some((_, value)) = kind.local_write() {
            visit(Self::Definition(value));
            return;
        }
        match kind {
            ValueKind::Phi { .. } => {}
            kind => kind.for_each_operand(|value| visit(Self::Reference(value))),
        }
    }
}

pub(crate) fn values(ir: &MethodIr, root: ValueUse) -> Values<'_> {
    Values::new(ir, vec![root])
}

pub(crate) fn statement_values<'a>(ir: &'a MethodIr, statement: &Statement) -> Values<'a> {
    let mut roots = Vec::new();
    for_each_statement_root(statement, |root| roots.push(root));
    Values::new(ir, roots)
}

/// Evaluated roots without traversal or deduplication, for consumers that need
/// occurrence counts or prune selected expression subtrees.
pub(crate) fn for_each_statement_root(statement: &Statement, mut visit: impl FnMut(ValueUse)) {
    match statement {
        Statement::Assign { value, .. } => visit(*value),
        Statement::Eval(value) => visit(ValueUse::Definition(*value)),
        _ => statement
            .operands()
            .into_iter()
            .for_each(|value| visit(ValueUse::Reference(value))),
    }
}

pub(crate) struct Values<'a> {
    ir: &'a MethodIr,
    pending: Vec<ValueUse>,
    seen: HashSet<ValueUse>,
}

impl<'a> Values<'a> {
    fn new(ir: &'a MethodIr, mut roots: Vec<ValueUse>) -> Self {
        roots.reverse();
        Self {
            ir,
            pending: roots,
            seen: HashSet::default(),
        }
    }

    /// Operation definitions, excluding reads of materialized locals.
    pub fn operations(self) -> impl Iterator<Item = ValueId> + 'a {
        let ir = self.ir;
        self.filter_map(move |usage| usage.local(ir).is_none().then_some(usage.value()))
    }
}

impl Iterator for Values<'_> {
    type Item = ValueUse;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(usage) = self.pending.pop() {
            if !self.seen.insert(usage) {
                continue;
            }
            let start = self.pending.len();
            usage.for_each_child(self.ir, |child| self.pending.push(child));
            self.pending[start..].reverse();
            return Some(usage);
        }
        None
    }
}

pub(crate) fn statement_may_throw(ir: &MethodIr, statement: &Statement) -> bool {
    !matches!(statement, Statement::Assign { .. } | Statement::Eval(_))
        || statement_values(ir, statement)
            .operations()
            .any(|value| ir.values[value].may_throw())
}

pub(crate) fn value_may_throw(ir: &MethodIr, usage: ValueUse) -> bool {
    values(ir, usage)
        .operations()
        .any(|value| ir.values[value].may_throw())
}

#[cfg(test)]
#[path = "../../tests/unit/scheduled.rs"]
mod tests;
