//! Canonical 0/1 producers in the scheduled source projection. A Z descriptor
//! gives Java syntax, not a proof about a parameter or a reused local slot.
//! This is a cached proof only: it never changes types, scheduling or values.
use crate::{
    classfile::JvmType,
    ir::{BinaryOp, Literal, MethodIr, ValueKind, scheduled::ValueUse, type_facts::ValueGraph},
};

pub(super) struct CanonicalBooleans {
    canonical: Vec<bool>,
}

impl CanonicalBooleans {
    pub fn build(ir: &MethodIr) -> Self {
        let values = ir.values.len();
        // Reverse edges invalidate copy/phi candidates when an input is unknown.
        let mut canonical = vec![false; values + ir.locals.len()];
        let mut graph = ValueGraph::new(ir);
        for (id, value) in ir.values.iter_enumerated() {
            canonical[id.index()] = match &value.kind {
                ValueKind::Literal(Literal::Int(0 | 1)) | ValueKind::InstanceOf { .. } => true,
                // A boolean invocation result has crossed its return boundary;
                // unlike an incoming argument slot, it is a boolean result.
                ValueKind::Call { .. } => value.ty == JvmType::Boolean,
                ValueKind::Alias(_) | ValueKind::InlineAssign { .. } | ValueKind::Select { .. } => {
                    true
                }
                ValueKind::Binary {
                    op: BinaryOp::And | BinaryOp::Or | BinaryOp::Xor,
                    left,
                    right,
                } => {
                    graph.require(ir, id.index(), ValueUse::Reference(*left));
                    graph.require(ir, id.index(), ValueUse::Reference(*right));
                    true
                }
                // Parameters and field/array descriptors describe source types,
                // not canonical producers. Keep legacy/external raw storage
                // distinct from a comparison that explicitly produces 0 or 1.
                _ => false,
            };
            // Keep this proof conservative over dormant embedded writes too;
            // range inference uses only the scheduled occurrences in the graph.
            if let Some((local, value)) = value.kind.local_write() {
                graph.require(ir, values + local.index(), ValueUse::Definition(value));
            }
        }
        for (local, binding) in ir.locals.iter_enumerated() {
            canonical[values + local.index()] = !graph.incoming[local]
                && (binding.needs_default || !graph.inputs[values + local.index()].is_empty());
        }
        // Start with possible copy/phi cycles and remove every cycle reached by
        // an unknown producer. Thus all writes to a loop-carried slot must be
        // canonical, including embedded assignments and reused source bindings.
        let users = graph.users();
        let mut pending: Vec<_> = canonical
            .iter()
            .enumerate()
            .filter_map(|(id, &known)| (!known).then_some(id))
            .collect();
        while let Some(input) = pending.pop() {
            for &dependent in &users[input] {
                if canonical[dependent] {
                    canonical[dependent] = false;
                    pending.push(dependent);
                }
            }
        }
        Self { canonical }
    }

    pub fn value(&self, ir: &MethodIr, usage: ValueUse) -> bool {
        if self.canonical[ValueGraph::node(ir, usage)] {
            return true;
        }
        // A literal 0/1 mask bounds the entire eager AND expression. Walk
        // only inline AND operands: a local or a Z call is not a mask proof.
        let mut pending = vec![usage];
        while let Some(usage) = pending.pop() {
            if usage.local(ir).is_some() {
                continue;
            }
            match ir.values[usage.value()].kind {
                ValueKind::Literal(Literal::Int(0 | 1)) => return true,
                ValueKind::Binary {
                    op: BinaryOp::And,
                    left,
                    right,
                } => pending.extend([ValueUse::Reference(left), ValueUse::Reference(right)]),
                _ => {}
            }
        }
        false
    }
}
