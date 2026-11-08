//! Canonical 0/1 producers in the scheduled source projection. A Z descriptor
//! gives Java syntax, not a proof about a parameter or a reused local slot.
//! Captured predicates keep their normalized value, so the scheduled producer
//! proof also describes the emitted storage without predicting source rewrites.
use crate::{
    classfile::JvmType,
    ir::{
        BinaryOp, Literal, MethodIr, ValueKind,
        scheduled::ValueUse,
        type_facts::{ValueGraph, has_boolean_mask},
    },
    program::{ClassId, MemberId, Program},
};

pub(super) struct CanonicalBooleans {
    canonical: Vec<bool>,
}

impl CanonicalBooleans {
    pub fn build(
        ir: &MethodIr,
        program: &Program<'_>,
        caller: Option<ClassId>,
        parameters: Option<MemberId>,
    ) -> Self {
        let values = ir.values.len();
        // Reverse edges invalidate copy/phi candidates when an input is unknown.
        let mut canonical = vec![false; values + ir.locals.len()];
        let mut graph = ValueGraph::new(ir);
        let mut incoming_sources = vec![false; ir.locals.len()];
        for (id, value) in ir.values.iter_enumerated() {
            canonical[id.index()] = match &value.kind {
                ValueKind::Literal(Literal::Int(0 | 1)) | ValueKind::InstanceOf { .. } => true,
                ValueKind::Parameter {
                    index: Some(index), ..
                } if value.ty == JvmType::Boolean => parameters
                    .is_some_and(|member| program.canonical_boolean_parameter(member, *index)),
                ValueKind::Field { field, .. } if value.ty == JvmType::Boolean => {
                    program.canonical_boolean_field(field)
                }
                // Legacy IRETURN may preserve raw ints. The descriptor alone
                // cannot certify either a local callee or unknown dispatch.
                ValueKind::Call { kind, method, .. } => {
                    value.ty == JvmType::Boolean
                        && program.canonical_boolean_return(caller, *kind, method)
                }
                ValueKind::Alias(_) | ValueKind::InlineAssign { .. } | ValueKind::Select { .. } => {
                    true
                }
                ValueKind::Binary {
                    op: BinaryOp::And, ..
                } if has_boolean_mask(ir, ValueUse::Definition(id)) => true,
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
            if matches!(
                value.kind,
                ValueKind::Parameter { .. } | ValueKind::CaughtException { .. }
            ) && let Some(local) = value.local
            {
                // The definition is the original incoming value; a scheduled
                // reference instead reads storage that later writes can reuse.
                graph.inputs[values + local.index()].push(id.index());
                incoming_sources[local.index()] = true;
            }
            // Keep this proof conservative over dormant embedded writes too;
            // range inference uses only the scheduled occurrences in the graph.
            if let Some((local, value)) = value.kind.local_write() {
                graph.require(ir, values + local.index(), ValueUse::Definition(value));
            }
        }
        for (local, binding) in ir.locals.iter_enumerated() {
            canonical[values + local.index()] = if graph.incoming[local] {
                incoming_sources[local.index()]
            } else {
                binding.needs_default || !graph.inputs[values + local.index()].is_empty()
            };
        }
        // Start with possible copy/phi cycles and remove every cycle reached by
        // an unknown producer. Thus all writes to a loop-carried slot must be
        // canonical, including embedded assignments and reused source bindings.
        propagate(&graph, &mut canonical);
        Self { canonical }
    }

    pub fn value(&self, ir: &MethodIr, usage: ValueUse) -> bool {
        self.canonical[ValueGraph::node(ir, usage)]
    }
}

fn propagate(graph: &ValueGraph, canonical: &mut [bool]) {
    let users = graph.users();
    let mut pending: Vec<_> = canonical
        .iter()
        .enumerate()
        .filter_map(|(id, &known)| (!known).then_some(id))
        .collect();
    while let Some(input) = pending.pop() {
        for &dependent in &users[input] {
            if std::mem::replace(&mut canonical[dependent], false) {
                pending.push(dependent);
            }
        }
    }
}
