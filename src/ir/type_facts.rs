//! Source-value dependencies and integral storage facts. A value definition
//! and a read of its mutable local are different nodes. Every physical write,
//! including an embedded conditional assignment, contributes to the local's
//! range before its Java declaration can become narrower.
use super::scheduled::ValueUse;
use super::{Literal, LocalId, MethodIr, Statement, Terminator, ValueKind, local_effects};
use crate::classfile::{JvmType, MethodDescriptor};
use crate::index::IdVec;
use std::collections::VecDeque;

/// Shared source-value dependencies, before an analysis chooses its seeds or
/// fixed point. Definition nodes and mutable local reads occupy separate slots.
pub(crate) struct ValueGraph {
    pub(crate) inputs: Vec<Vec<usize>>,
    /// Caller/exception bindings remain incoming after local coalescing.
    pub(crate) incoming: IdVec<LocalId, bool>,
}

impl ValueGraph {
    pub(crate) fn new(ir: &MethodIr) -> Self {
        let mut graph = Self {
            inputs: vec![Vec::new(); ir.values.len() + ir.locals.len()],
            incoming: ir
                .locals
                .iter()
                .map(|local| local.is_this || local.parameter.is_some())
                .collect(),
        };
        for (id, value) in ir.values.iter_enumerated() {
            match &value.kind {
                ValueKind::Alias(value) => {
                    graph.require(ir, id.index(), ValueUse::Reference(*value))
                }
                ValueKind::InlineAssign { value, .. } => {
                    graph.require(ir, id.index(), ValueUse::Definition(*value))
                }
                ValueKind::Select {
                    then_value,
                    else_value,
                    ..
                } => {
                    graph.require(ir, id.index(), ValueUse::Reference(*then_value));
                    graph.require(ir, id.index(), ValueUse::Reference(*else_value));
                }
                ValueKind::Parameter { .. } | ValueKind::CaughtException { .. } => {
                    if let Some(local) = value.local {
                        graph.incoming[local] = true;
                    }
                }
                _ => {}
            }
        }
        let mut observe = |local: LocalId, value| {
            graph.require(ir, ir.values.len() + local.index(), value);
        };
        for block in &ir.blocks {
            for statement in &block.statements {
                local_effects::statement(ir, statement, false).assignments(&mut observe);
            }
            local_effects::terminator(ir, &block.terminator, false).assignments(&mut observe);
        }
        graph
    }

    pub(crate) fn node(ir: &MethodIr, usage: ValueUse) -> usize {
        usage.local(ir).map_or(usage.value().index(), |local| {
            ir.values.len() + local.index()
        })
    }

    pub(crate) fn require(&mut self, ir: &MethodIr, dependent: usize, usage: ValueUse) {
        self.inputs[dependent].push(Self::node(ir, usage));
    }

    pub(crate) fn users(&self) -> Vec<Vec<usize>> {
        let mut users = vec![Vec::new(); self.inputs.len()];
        for (id, inputs) in self.inputs.iter().enumerate() {
            for &input in inputs {
                users[input].push(id);
            }
        }
        users
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IntegralRange {
    pub(crate) min: i32,
    pub(crate) max: i32,
}

impl IntegralRange {
    const INT: Self = Self {
        min: i32::MIN,
        max: i32::MAX,
    };

    pub(crate) fn of_type(ty: &JvmType) -> Option<Self> {
        let (min, max) = match ty {
            JvmType::Byte => (i8::MIN.into(), i8::MAX.into()),
            JvmType::Short => (i16::MIN.into(), i16::MAX.into()),
            JvmType::Char => (0, u16::MAX.into()),
            JvmType::Int => (i32::MIN, i32::MAX),
            _ => return None,
        };
        Some(Self { min, max })
    }

    pub(crate) fn fits(self, ty: &JvmType) -> bool {
        Self::of_type(ty).is_some_and(|range| range.min <= self.min && self.max <= range.max)
    }

    fn join(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Fact {
    range: Option<IntegralRange>,
    // A fitting constant alone is not a reason to turn every int counter into
    // byte storage. Keep source intent from narrow producers or typed consumers.
    narrow: u8,
}

impl Fact {
    fn typed(ty: &JvmType) -> Self {
        Self {
            range: IntegralRange::of_type(ty),
            narrow: match ty {
                JvmType::Byte => 1,
                JvmType::Short => 2,
                JvmType::Char => 4,
                _ => 0,
            },
        }
    }

    fn constant(value: i32) -> Self {
        Self {
            range: Some(IntegralRange {
                min: value,
                max: value,
            }),
            narrow: 0,
        }
    }

    fn join(self, other: Self) -> Self {
        Self {
            range: match (self.range, other.range) {
                (Some(left), Some(right)) => Some(left.join(right)),
                (left, right) => left.or(right),
            },
            narrow: self.narrow | other.narrow,
        }
    }
}

/// Reusable range facts retain their value-definition/read distinction. They
/// neither change JVM values nor treat a declared boolean as a canonical value.
pub(crate) struct IntegralFacts {
    values: usize,
    facts: Vec<Fact>,
}

impl IntegralFacts {
    pub(crate) fn analyze(ir: &MethodIr) -> Self {
        let values = ir.values.len();
        let count = values + ir.locals.len();
        let mut facts = vec![Fact::default(); count];
        let mut graph = ValueGraph::new(ir);
        for (id, value) in ir.values.iter_enumerated() {
            // The fact graph packs value and local nodes into one solver space.
            let id = id.index();
            match &value.kind {
                ValueKind::Literal(Literal::Int(value)) => facts[id] = Fact::constant(*value),
                ValueKind::Alias(_) | ValueKind::InlineAssign { .. } | ValueKind::Select { .. } => {
                }
                ValueKind::LocalUpdate { local, .. } => {
                    graph.inputs[id].push(values + local.index())
                }
                ValueKind::Cast { ty, .. }
                | ValueKind::ArrayLoad {
                    element_type: ty, ..
                } => facts[id] = Fact::typed(ty),
                ValueKind::Call { .. }
                | ValueKind::Field { .. }
                | ValueKind::Update { .. }
                | ValueKind::CompoundUpdate { .. } => {
                    facts[id] = Fact::typed(&value.ty);
                }
                _ if value.ty.is_int_like() => {
                    // In particular, a B/C/S/Z parameter descriptor does not
                    // prove the contents of a raw JVM incoming argument slot.
                    facts[id].range = Some(IntegralRange::INT);
                }
                _ => {}
            }
        }
        for (id, local) in ir.locals.iter_enumerated() {
            if !local.ty.is_int_like() {
                continue;
            }
            if graph.incoming[id] || graph.inputs[values + id.index()].is_empty() {
                facts[values + id.index()].range = Some(IntegralRange::INT);
            } else if local.needs_default {
                facts[values + id.index()] = Fact::constant(0);
            }
        }
        let mut demand = |value, ty: &JvmType| {
            facts[ValueGraph::node(ir, ValueUse::Reference(value))].narrow |=
                Fact::typed(ty).narrow;
        };
        for value in &ir.values {
            match &value.kind {
                ValueKind::Cast { value, ty } => demand(*value, ty),
                ValueKind::Call { method, args, .. } => {
                    if let Ok(signature) = MethodDescriptor::parse(&method.descriptor) {
                        for (&value, ty) in args.iter().zip(&signature.parameters) {
                            demand(value, ty);
                        }
                    }
                }
                ValueKind::New {
                    descriptor, args, ..
                } => {
                    if let Ok(signature) = MethodDescriptor::parse(descriptor) {
                        for (&value, ty) in args.iter().zip(&signature.parameters) {
                            demand(value, ty);
                        }
                    }
                }
                _ => {}
            }
        }
        for block in &ir.blocks {
            for statement in &block.statements {
                match statement {
                    Statement::PutField { field, value, .. } => {
                        if let Ok(ty) = JvmType::parse(&field.descriptor) {
                            demand(*value, &ty);
                        }
                    }
                    Statement::ArrayStore {
                        value,
                        element_type,
                        ..
                    } => demand(*value, element_type),
                    Statement::ConstructorCall { method, args, .. } => {
                        if let Ok(signature) = MethodDescriptor::parse(&method.descriptor) {
                            for (&value, ty) in args.iter().zip(&signature.parameters) {
                                demand(value, ty);
                            }
                        }
                    }
                    _ => {}
                }
            }
            if let Terminator::Return(Some(value)) = block.terminator {
                demand(value, &ir.return_type);
            }
        }
        let users = graph.users();
        let mut pending: VecDeque<_> = (0..count).collect();
        let mut queued = vec![true; count];
        while let Some(id) = pending.pop_front() {
            queued[id] = false;
            let next = graph.inputs[id]
                .iter()
                .fold(facts[id], |fact, &input| fact.join(facts[input]));
            if next != facts[id] {
                facts[id] = next;
                for &user in &users[id] {
                    if !queued[user] {
                        queued[user] = true;
                        pending.push_back(user);
                    }
                }
            }
        }
        // Typed use sites can reveal byte storage consisting entirely of small
        // constants. Carry that preference backward only through the same copy
        // dependencies; the independently joined range must still fit it.
        let mut pending: Vec<_> = (0..count).filter(|&id| facts[id].narrow != 0).collect();
        while let Some(id) = pending.pop() {
            for &input in &graph.inputs[id] {
                let narrow = facts[input].narrow | facts[id].narrow;
                if narrow != facts[input].narrow {
                    facts[input].narrow = narrow;
                    pending.push(input);
                }
            }
        }
        Self { values, facts }
    }

    pub(crate) fn local_range(&self, local: LocalId) -> Option<IntegralRange> {
        self.facts[self.values + local.index()].range
    }

    fn narrow_type(&self, local: LocalId) -> Option<JvmType> {
        let range = self.local_range(local)?;
        let narrow = self.facts[self.values + local.index()].narrow;
        [(1, JvmType::Byte), (2, JvmType::Short), (4, JvmType::Char)]
            .into_iter()
            .find_map(|(bit, ty)| (narrow & bit != 0 && range.fits(&ty)).then_some(ty))
    }
}

pub(super) fn refine_integral_locals(ir: &mut MethodIr) {
    let facts = IntegralFacts::analyze(ir);
    for (id, local) in ir.locals.iter_enumerated_mut() {
        if local.ty == JvmType::Int
            && local.parameter.is_none()
            && !local.is_this
            && let Some(ty) = facts.narrow_type(id)
        {
            local.ty = ty;
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/integral_type_facts.rs"]
mod tests;

pub(crate) fn widening(from: &JvmType, to: &JvmType) -> bool {
    use JvmType::*;
    from == to
        || matches!(
            (from, to),
            (Byte, Short | Int | Long | Float | Double)
                | (Short | Char, Int | Long | Float | Double)
                | (Int, Long | Float | Double)
                | (Long, Float | Double)
                | (Float, Double)
        )
}

pub(crate) fn numeric(ty: &JvmType) -> bool {
    matches!(
        ty,
        JvmType::Byte
            | JvmType::Short
            | JvmType::Char
            | JvmType::Int
            | JvmType::Long
            | JvmType::Float
            | JvmType::Double
    )
}

pub(crate) fn promoted(left: &JvmType, right: &JvmType) -> Option<JvmType> {
    if !numeric(left) || !numeric(right) {
        return None;
    }
    Some(if *left == JvmType::Double || *right == JvmType::Double {
        JvmType::Double
    } else if *left == JvmType::Float || *right == JvmType::Float {
        JvmType::Float
    } else if *left == JvmType::Long || *right == JvmType::Long {
        JvmType::Long
    } else {
        JvmType::Int
    })
}
