//! Small interval and residue proofs for array layouts. JVM overflow is part of
//! the calculation: only powers of two divide the int overflow modulus.
use super::flow::{Graph, KeyRoot, NodeId, Op};
use crate::classfile::JvmType;
use crate::ir::{BinaryOp, Literal, LocalId};
use crate::mappings::RecordLayout;
use std::{
    cell::RefCell,
    collections::{BTreeSet, HashSet, VecDeque},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Range {
    pub min: i64,
    pub max: i64,
}
impl Range {
    fn full(ty: &JvmType) -> Self {
        let (min, max) = match ty {
            JvmType::Byte => (i8::MIN.into(), i8::MAX.into()),
            JvmType::Short => (i16::MIN.into(), i16::MAX.into()),
            JvmType::Char => (0, u16::MAX.into()),
            JvmType::Long => (i64::MIN, i64::MAX),
            _ => (i32::MIN.into(), i32::MAX.into()),
        };
        Self { min, max }
    }
    // B/C/S/Z descriptors do not narrow an incoming JVM local. Arithmetic also
    // executes at int width, irrespective of the printable local's type.
    fn runtime(ty: &JvmType) -> Self {
        Self::full(if *ty == JvmType::Long {
            &JvmType::Long
        } else {
            &JvmType::Int
        })
    }
    fn contains(self, other: Self) -> bool {
        other.min > other.max || self.min <= other.min && self.max >= other.max
    }
    fn join(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }
}

/// One proof context borrows a frozen occurrence graph. Limits and completed
/// acyclic proofs are shared by every layout/cast query and solver revisit;
/// cycle-dependent results remain specific to the current query entry point.
pub(super) struct Queries<'g> {
    pub graph: &'g Graph,
    pub induction: Vec<Option<Induction>>,
    state: RefCell<Option<RangeQuery>>,
}
impl<'g> Queries<'g> {
    pub fn new(graph: &'g Graph) -> Self {
        Self {
            graph,
            induction: (0..graph.nodes.len())
                .map(|node| induction(graph, node))
                .collect(),
            state: RefCell::new(None),
        }
    }
    pub fn range(&self, value: NodeId) -> Range {
        self.state
            .borrow_mut()
            .get_or_insert_with(|| RangeQuery {
                limits: range_limits(self.graph),
                cached: vec![None; self.graph.nodes.len()],
                visiting: vec![false; self.graph.nodes.len()],
                cycles: 0,
            })
            .range(self.graph, &self.induction, value)
    }
}

#[cfg(test)]
fn range(graph: &Graph, value: NodeId) -> Range {
    Queries::new(graph).range(value)
}

/// Bounds supplied directly by operations, before recursive arithmetic proofs.
/// Only transparent joins propagate these bounds, so their worklist draws its
/// endpoints from a finite set and cannot chase an induction one step at a time.
fn range_limits(graph: &Graph) -> Vec<Range> {
    let mut limits: Vec<_> = graph
        .nodes
        .iter()
        .map(|node| match &node.op {
            Op::Literal(Literal::Int(value)) => Range {
                min: i64::from(*value),
                max: i64::from(*value),
            },
            Op::Literal(Literal::Long(value)) => Range {
                min: *value,
                max: *value,
            },
            Op::Cast { ty, .. } => Range::full(ty),
            Op::Field(_) | Op::ArrayLoad { .. } => Range::full(&node.ty),
            Op::ArrayLength(_) => Range {
                min: 0,
                max: i32::MAX.into(),
            },
            _ => Range::runtime(&node.ty),
        })
        .collect();
    let mut readers = vec![Vec::new(); graph.nodes.len()];
    let mut sources = vec![Vec::new(); graph.nodes.len()];
    for (node, data) in graph.nodes.iter().enumerate() {
        sources[node] = match &data.op {
            Op::Copy(source) => vec![*source],
            Op::Read { sources, .. } => sources.iter().copied().collect(),
            Op::Select { yes, no, .. } => vec![*yes, *no],
            _ => Vec::new(),
        };
        for &source in &sources[node] {
            readers[source].push(node);
        }
    }
    let mut queue: VecDeque<_> = (0..graph.nodes.len()).collect();
    let mut queued = vec![true; graph.nodes.len()];
    while let Some(node) = queue.pop_front() {
        queued[node] = false;
        let Some(bound) = sources[node]
            .iter()
            .map(|&source| limits[source])
            .reduce(Range::join)
        else {
            continue;
        };
        let next = Range {
            min: limits[node].min.max(bound.min),
            max: limits[node].max.min(bound.max),
        };
        if next != limits[node] {
            limits[node] = next;
            for &reader in &readers[node] {
                if !queued[reader] {
                    queued[reader] = true;
                    queue.push_back(reader);
                }
            }
        }
    }
    limits
}

struct RangeQuery {
    limits: Vec<Range>,
    cached: Vec<Option<Range>>,
    visiting: Vec<bool>,
    cycles: usize,
}

impl RangeQuery {
    fn range(&mut self, graph: &Graph, induction: &[Option<Induction>], value: NodeId) -> Range {
        if let Some(range) = self.cached[value] {
            return range;
        }
        let fallback = self.limits[value];
        if self.visiting[value] {
            self.cycles += 1;
            return fallback;
        }
        self.visiting[value] = true;
        let cycles = self.cycles;
        let result = if let Some(number) = graph.integer(value) {
            Range {
                min: number,
                max: number,
            }
        } else if let Some(counter) = &induction[value] {
            counter.range
        } else {
            match &graph.nodes[value].op {
                Op::Copy(source) => self.range(graph, induction, *source),
                Op::Read { sources, .. } => {
                    let mut result: Option<Range> = None;
                    for &source in sources {
                        // Once an alternative's entire possible range is present,
                        // following its paths cannot enlarge this join. In a loop
                        // with many narrowing updates, this avoids enumerating
                        // every permutation of the same reaching definitions.
                        if result.is_some_and(|range| range.contains(self.limits[source])) {
                            continue;
                        }
                        let next = self.range(graph, induction, source);
                        result = Some(result.map_or(next, |range| range.join(next)));
                    }
                    result.unwrap_or(fallback)
                }
                Op::Cast { value: source, ty } => {
                    let source = self.range(graph, induction, *source);
                    let target = Range::full(ty);
                    if source.min >= target.min && source.max <= target.max {
                        source
                    } else {
                        target
                    }
                }
                Op::Field(_) | Op::ArrayLoad { .. } => Range::full(&graph.nodes[value].ty),
                Op::ArrayLength(_) => Range {
                    min: 0,
                    max: i32::MAX.into(),
                },
                Op::Select { yes, no, .. } => {
                    let yes = self.range(graph, induction, *yes);
                    if yes.contains(self.limits[*no]) {
                        yes
                    } else {
                        yes.join(self.range(graph, induction, *no))
                    }
                }
                Op::Binary {
                    op: BinaryOp::And,
                    left,
                    right,
                } => {
                    let pair = graph
                        .integer(*right)
                        .map(|mask| (*left, mask))
                        .or_else(|| graph.integer(*left).map(|mask| (*right, mask)));
                    pair.map_or(fallback, |(source, mask)| {
                        let mask = if graph.nodes[value].ty == JvmType::Long {
                            mask
                        } else {
                            i64::from(mask as i32)
                        };
                        if mask == -1 {
                            self.range(graph, induction, source)
                        } else if mask >= 0 {
                            Range { min: 0, max: mask }
                        } else {
                            let source = self.range(graph, induction, source);
                            Range {
                                min: if source.min >= 0 { 0 } else { fallback.min },
                                max: mask & fallback.max,
                            }
                        }
                    })
                }
                Op::Binary { op, left, right } => arithmetic(
                    *op,
                    self.range(graph, induction, *left),
                    self.range(graph, induction, *right),
                    &graph.nodes[value].ty,
                )
                .unwrap_or(fallback),
                _ => fallback,
            }
        };
        self.visiting[value] = false;
        let mut result = result;
        if let Some(key) = graph.key(value) {
            for guard in &graph.nodes[value].guards {
                if guard.key != key {
                    continue;
                }
                match guard.relation {
                    0 => {
                        result.min = result.min.max(guard.number);
                        result.max = result.max.min(guard.number);
                    }
                    2 => {
                        if let Some(max) = guard.number.checked_sub(1) {
                            result.max = result.max.min(max);
                        } else {
                            result = Range { min: 1, max: 0 };
                            break;
                        }
                    }
                    3 => result.max = result.max.min(guard.number),
                    4 => {
                        if let Some(min) = guard.number.checked_add(1) {
                            result.min = result.min.max(min);
                        } else {
                            result = Range { min: 1, max: 0 };
                            break;
                        }
                    }
                    5 => result.min = result.min.max(guard.number),
                    _ => {}
                }
            }
        }
        // A proof which used an ancestor's cycle bound can be more precise from
        // another entry point. Cache only proofs independent of that context.
        if self.cycles == cycles {
            self.cached[value] = Some(result);
        }
        result
    }
}

fn arithmetic(op: BinaryOp, left: Range, right: Range, ty: &JvmType) -> Option<Range> {
    if left.min > left.max || right.min > right.max {
        return None;
    }
    let left = (i128::from(left.min), i128::from(left.max));
    let right = (i128::from(right.min), i128::from(right.max));
    let (min, max) = match op {
        BinaryOp::Add => (left.0 + right.0, left.1 + right.1),
        BinaryOp::Sub => (left.0 - right.1, left.1 - right.0),
        BinaryOp::Mul => {
            let values = [
                left.0 * right.0,
                left.0 * right.1,
                left.1 * right.0,
                left.1 * right.1,
            ];
            (*values.iter().min()?, *values.iter().max()?)
        }
        BinaryOp::Div if right.0 == right.1 && right.0 != 0 => {
            let values = [left.0 / right.0, left.1 / right.0];
            (*values.iter().min()?, *values.iter().max()?)
        }
        BinaryOp::Rem if right.0 == right.1 && right.0 != 0 => {
            let limit = right.0.abs() - 1;
            (
                if left.0 < 0 { left.0.max(-limit) } else { 0 },
                if left.1 > 0 { left.1.min(limit) } else { 0 },
            )
        }
        BinaryOp::Shl if right.0 == right.1 => {
            let shift = (right.0 as u32) & if *ty == JvmType::Long { 63 } else { 31 };
            (left.0 << shift, left.1 << shift)
        }
        BinaryOp::Shr if right.0 == right.1 => {
            let shift = (right.0 as u32) & if *ty == JvmType::Long { 63 } else { 31 };
            (left.0 >> shift, left.1 >> shift)
        }
        BinaryOp::Ushr if right.0 == right.1 => {
            let bits = if *ty == JvmType::Long { 64 } else { 32 };
            let shift = (right.0 as u32) & (bits - 1);
            if shift == 0 {
                left
            } else if left.0 >= 0 {
                (left.0 >> shift, left.1 >> shift)
            } else if left.1 < 0 {
                (
                    (left.0 + (1i128 << bits)) >> shift,
                    (left.1 + (1i128 << bits)) >> shift,
                )
            } else {
                (0, ((1i128 << bits) - 1) >> shift)
            }
        }
        _ => return None,
    };
    let target = Range::runtime(ty);
    (min >= i128::from(target.min) && max <= i128::from(target.max)).then_some(Range {
        min: min as i64,
        max: max as i64,
    })
}

/// A recurrence established at this particular body read. The loop's test and
/// exit reads have no qualifying body guard and deliberately get no such fact.
#[derive(Clone, Debug)]
pub(super) struct Induction {
    pub root: KeyRoot,
    pub start: i64,
    pub step: i64,
    pub range: Range,
}

pub(super) fn induction(graph: &Graph, node: NodeId) -> Option<Induction> {
    let Op::Read { local, sources } = &graph.nodes[node].op else {
        return None;
    };
    let root = KeyRoot::Local(*local, sources.clone());
    let mut start = None;
    let mut updates = Vec::new();
    let mut step = None;
    for &source in sources {
        if let Some(number) = graph.integer(source) {
            if number < 0 || start.is_some_and(|old| old != number) {
                return None;
            }
            start = Some(number);
        } else {
            let update = counter_update(graph, source, *local, sources)?;
            if update.step == 0 || step.is_some_and(|old| old != update.step) {
                return None;
            }
            step = Some(update.step);
            updates.push(update);
        }
    }
    let start = start?;
    let step = step?;
    for update in &updates {
        let body = counter_bounds(graph, update.read, &root, start, step, &[])?;
        let next = if step > 0 {
            i128::from(body.max) + i128::from(step)
        } else {
            i128::from(body.min) + i128::from(step)
        };
        // A signed positive update may wrap only into negative values. If
        // every next update and this read require a nonnegative cursor, those
        // wrapped values cannot continue the recurrence. Successful array
        // accesses supply exactly this gate, without constraining a handler.
        let gated = step > 0
            && step <= update.storage.max
            && update.storage.min < 0
            && nonnegative_guard(graph, node, &root)
            && updates
                .iter()
                .all(|update| nonnegative_guard(graph, update.read, &root));
        // A narrow local declaration alone is not a narrowing operation.
        if start < update.storage.min
            || start > update.storage.max
            || next < i128::from(update.storage.min)
            || next > i128::from(update.storage.max) && !gated
        {
            return None;
        }
    }
    // A new range test after the loop must not inherit the body's index
    // interpretation. Require the actual bound that protected each update.
    let update_reads: Vec<_> = updates.iter().map(|update| update.read).collect();
    let range = counter_bounds(graph, node, &root, start, step, &update_reads)?;
    (range.min >= 0).then_some(Induction {
        root,
        start,
        step,
        range,
    })
}

fn nonnegative_guard(graph: &Graph, node: NodeId, root: &KeyRoot) -> bool {
    graph.nodes[node].guards.iter().any(|guard| {
        guard.key.root == *root
            && guard.key.mask.is_none()
            && match guard.relation {
                0 | 5 => guard.number >= 0,
                4 => guard.number >= -1,
                _ => false,
            }
    })
}

struct CounterUpdate {
    read: NodeId,
    step: i64,
    storage: Range,
}
fn counter_update(
    graph: &Graph,
    mut node: NodeId,
    local: LocalId,
    sources: &BTreeSet<NodeId>,
) -> Option<CounterUpdate> {
    let mut storage = Range::runtime(&graph.nodes[node].ty);
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(node) {
            return None;
        }
        match &graph.nodes[node].op {
            Op::Copy(value) => node = *value,
            Op::Cast { value, ty } if ty.is_int_like() || *ty == JvmType::Long => {
                let target = Range::full(ty);
                storage.min = storage.min.max(target.min);
                storage.max = storage.max.min(target.max);
                node = *value;
            }
            _ => break,
        }
    }
    let runtime = Range::runtime(&graph.nodes[node].ty);
    storage.min = storage.min.max(runtime.min);
    storage.max = storage.max.min(runtime.max);
    let Op::Binary { op, left, right } = graph.nodes[node].op else {
        return None;
    };
    let (read, step) = match op {
        BinaryOp::Add => graph
            .integer(right)
            .map(|step| (left, step))
            .or_else(|| graph.integer(left).map(|step| (right, step)))?,
        BinaryOp::Sub => (left, graph.integer(right)?.checked_neg()?),
        _ => return None,
    };
    let mut read = read;
    while let Op::Copy(value) = graph.nodes[read].op {
        if !seen.insert(read) {
            return None;
        }
        read = value;
    }
    match &graph.nodes[read].op {
        Op::Read {
            local: actual,
            sources: definitions,
        } if *actual == local && definitions == sources => Some(CounterUpdate {
            read,
            step,
            storage,
        }),
        _ => None,
    }
}

fn counter_bounds(
    graph: &Graph,
    node: NodeId,
    root: &KeyRoot,
    start: i64,
    step: i64,
    shared_with: &[NodeId],
) -> Option<Range> {
    let mut bound = None;
    for guard in &graph.nodes[node].guards {
        if guard.key.root != *root || guard.key.mask.is_some() {
            continue;
        }
        if shared_with
            .iter()
            .any(|&other| !graph.nodes[other].guards.contains(guard))
        {
            continue;
        }
        let limit = match (step > 0, guard.relation) {
            (true, 2) => guard.number.checked_sub(1),
            (true, 3) | (_, 0) | (false, 5) => Some(guard.number),
            (false, 4) => guard.number.checked_add(1),
            (_, 1)
                if (i128::from(guard.number) - i128::from(start)) % i128::from(step) == 0
                    && (guard.number > start) == (step > 0) =>
            {
                guard.number.checked_sub(step)
            }
            _ => None,
        };
        if let Some(limit) = limit {
            bound = Some(bound.map_or(limit, |old: i64| {
                if step > 0 {
                    old.min(limit)
                } else {
                    old.max(limit)
                }
            }));
        }
    }
    let bound = bound?;
    let distance = if step > 0 {
        i128::from(bound) - i128::from(start)
    } else {
        i128::from(start) - i128::from(bound)
    };
    if distance < 0 {
        return None;
    }
    let end = i128::from(start) + distance / i128::from(step).abs() * i128::from(step);
    let end = i64::try_from(end).ok()?;
    Some(if step > 0 {
        Range {
            min: start,
            max: end,
        }
    } else {
        Range {
            min: end,
            max: start,
        }
    })
}

pub(super) fn slot(queries: &Queries<'_>, index: NodeId, layout: &RecordLayout) -> Option<u32> {
    let graph = queries.graph;
    let stride = i64::from(layout.stride);
    if stride == 0 {
        return None;
    }
    let offset = i64::from(layout.offset);
    if let Some(index) = graph.integer(index) {
        return (index >= offset)
            .then(|| {
                if layout.planes {
                    (index - offset) / stride
                } else {
                    (index - offset) % stride
                }
            })
            .and_then(|slot| slot.try_into().ok());
    }
    let range = queries.range(index);
    let min = range.min.max(0);
    if range.max < min {
        return None;
    }
    if layout.planes {
        if min < offset {
            return None;
        }
        let first = (min - offset) / stride;
        let last = (range.max - offset) / stride;
        return (first == last).then_some(first as u32);
    }
    let residue = residue(queries, index, stride, &mut HashSet::new())?;
    let first = min + (residue - min).rem_euclid(stride);
    (first >= offset && first <= range.max).then_some((residue - offset).rem_euclid(stride) as u32)
}

fn residue(
    queries: &Queries<'_>,
    value: NodeId,
    stride: i64,
    visiting: &mut HashSet<NodeId>,
) -> Option<i64> {
    residue_inner(queries, value, stride, visiting, &mut Vec::new())
}

fn residue_inner(
    queries: &Queries<'_>,
    value: NodeId,
    stride: i64,
    visiting: &mut HashSet<NodeId>,
    assumptions: &mut Vec<(KeyRoot, i64)>,
) -> Option<i64> {
    let graph = queries.graph;
    if stride == 1 {
        return Some(0);
    }
    if let Op::Read { local, sources } = &graph.nodes[value].op {
        let root = KeyRoot::Local(*local, sources.clone());
        if let Some((_, residue)) = assumptions.iter().find(|(key, _)| *key == root) {
            return Some(*residue);
        }
    }
    if !visiting.insert(value) {
        return None;
    }
    let result = if let Some(value) = graph.integer(value) {
        Some(value.rem_euclid(stride))
    } else if let Some(counter) = &queries.induction[value] {
        (counter.step.rem_euclid(stride) == 0 || counter.range.min == counter.range.max)
            .then_some(counter.start.rem_euclid(stride))
    } else {
        match &graph.nodes[value].op {
            Op::Copy(value) => residue_inner(queries, *value, stride, visiting, assumptions),
            Op::Cast { value: source, ty } => {
                let source_range = queries.range(*source);
                let target = Range::full(ty);
                let bits = match ty {
                    JvmType::Byte => 8,
                    JvmType::Short | JvmType::Char => 16,
                    JvmType::Int => 32,
                    JvmType::Long => 64,
                    _ => 0,
                };
                if source_range.min >= target.min && source_range.max <= target.max
                    || bits != 0 && (1u128 << bits).is_multiple_of(stride as u128)
                {
                    residue_inner(queries, *source, stride, visiting, assumptions)
                } else {
                    None
                }
            }
            Op::Read { local, sources } => {
                // First establish a seed without assuming this read's residue.
                // Then every reaching definition, including every backedge,
                // must preserve it. This proves congruence only: a secondary
                // record cursor may wrap and has no inferred bounds or domain.
                let seed = sources.iter().find_map(|&source| {
                    residue_inner(queries, source, stride, visiting, assumptions)
                });
                seed.and_then(|seed| {
                    assumptions.push((KeyRoot::Local(*local, sources.clone()), seed));
                    let agrees = sources.iter().all(|&source| {
                        residue_inner(queries, source, stride, visiting, assumptions) == Some(seed)
                    });
                    assumptions.pop();
                    agrees.then_some(seed)
                })
            }
            Op::Select { yes, no, .. } => {
                residue_inner(queries, *yes, stride, visiting, assumptions)
                    .zip(residue_inner(queries, *no, stride, visiting, assumptions))
                    .filter(|(yes, no)| yes == no)
                    .map(|(value, _)| value)
            }
            Op::Binary { op, left, right } => {
                let can_wrap = !(stride as u64).is_power_of_two() && {
                    let left_range = queries.range(*left);
                    let right_range = queries.range(*right);
                    // Two nonnegative signed operands can overflow addition
                    // only into a negative result. A checked index excludes
                    // that result even when the operand bounds are too wide.
                    let checked_add = *op == BinaryOp::Add
                        && left_range.min >= 0
                        && right_range.min >= 0
                        && queries.range(value).min >= 0;
                    arithmetic(*op, left_range, right_range, &graph.nodes[value].ty).is_none()
                        && !checked_add
                };
                if can_wrap {
                    None
                } else {
                    let left_residue = residue_inner(queries, *left, stride, visiting, assumptions);
                    let right_residue =
                        residue_inner(queries, *right, stride, visiting, assumptions);
                    match op {
                        BinaryOp::Add => left_residue
                            .zip(right_residue)
                            .map(|(left, right)| (left + right).rem_euclid(stride)),
                        BinaryOp::Sub => left_residue
                            .zip(right_residue)
                            .map(|(left, right)| (left - right).rem_euclid(stride)),
                        BinaryOp::Mul if left_residue == Some(0) || right_residue == Some(0) => {
                            Some(0)
                        }
                        BinaryOp::Mul => left_residue.zip(right_residue).map(|(left, right)| {
                            ((i128::from(left) * i128::from(right)) % i128::from(stride)) as i64
                        }),
                        BinaryOp::And
                            if (stride as u64).is_power_of_two()
                                && (left_residue == Some(0) || right_residue == Some(0)) =>
                        {
                            Some(0)
                        }
                        BinaryOp::And if (stride as u64).is_power_of_two() => left_residue
                            .zip(right_residue)
                            .map(|(left, right)| left & right),
                        BinaryOp::Shl => graph.integer(*right).and_then(|shift| {
                            let factor = 1i128
                                << (shift as u32
                                    & if graph.nodes[value].ty == JvmType::Long {
                                        63
                                    } else {
                                        31
                                    });
                            if factor % i128::from(stride) == 0 {
                                Some(0)
                            } else {
                                left_residue.map(|left| {
                                    ((i128::from(left) * factor) % i128::from(stride)) as i64
                                })
                            }
                        }),
                        _ => None,
                    }
                }
            }
            _ => None,
        }
    };
    visiting.remove(&value);
    result
}

pub(super) fn offset_literal(
    queries: &Queries<'_>,
    mut index: NodeId,
    layout: &RecordLayout,
    slot: u32,
) -> Option<NodeId> {
    let graph = queries.graph;
    if layout.planes || layout.stride == 0 {
        return None;
    }
    if layout.offset == 0 && graph.integer(index) == Some(i64::from(slot)) {
        return Some(index);
    }
    // A captured address still names the offset in its defining expression.
    // Multiple reaching definitions need separate literal demands; leave them
    // alone rather than choosing one producer arbitrarily.
    let mut visiting = HashSet::new();
    loop {
        if !visiting.insert(index) {
            return None;
        }
        index = match &graph.nodes[index].op {
            Op::Copy(source) => *source,
            Op::Read { sources, .. } if sources.len() == 1 => *sources.first()?,
            _ => break,
        };
    }
    if let Op::Binary {
        op: BinaryOp::Add,
        left,
        right,
    } = graph.nodes[index].op
    {
        for (literal, base) in [(right, left), (left, right)] {
            if graph.integer(literal) == Some(i64::from(slot))
                && residue(queries, base, i64::from(layout.stride), &mut HashSet::new())
                    == Some(i64::from(layout.offset % layout.stride))
            {
                return Some(literal);
            }
        }
    }
    None
}

/// Each tuple identifies a literal and the equal expression slot * stride +
/// addend. The header may be a separate term, as in header + i + planeBase.
pub(super) fn plane_offsets(
    graph: &Graph,
    index: NodeId,
    layout: &RecordLayout,
) -> Vec<(NodeId, i64, i64)> {
    if !layout.planes || layout.stride == 0 {
        return Vec::new();
    }
    fn terms(graph: &Graph, node: NodeId, constants: &mut Vec<(NodeId, i64)>) {
        if let Some(number) = graph.integer(node) {
            constants.push((node, number));
        } else if let Op::Binary {
            op: BinaryOp::Add,
            left,
            right,
        } = graph.nodes[node].op
        {
            terms(graph, left, constants);
            terms(graph, right, constants);
        }
    }
    let mut constants = Vec::new();
    terms(graph, index, &mut constants);
    let total: i128 = constants
        .iter()
        .map(|(_, number)| i128::from(*number))
        .sum();
    let stride = i64::from(layout.stride);
    let offset = i64::from(layout.offset);
    constants
        .into_iter()
        .filter_map(|(node, number)| {
            let adjusted = number.checked_sub(offset)?;
            if adjusted >= 0 && adjusted % stride == 0 {
                Some((node, adjusted / stride, offset))
            } else if number >= 0
                && number % stride == 0
                && total - i128::from(number) == i128::from(offset)
            {
                Some((node, number / stride, 0))
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
pub(super) fn test_graph(ops: Vec<(JvmType, Op)>) -> Graph {
    use super::{Site, flow::Node};
    use crate::{
        index::IdVec,
        ir::{BlockId, IrBlock, MethodIr, Terminator},
    };
    let block = BlockId::from_index(0);
    let ir = MethodIr {
        entry: block,
        values: IdVec::new(),
        blocks: vec![IrBlock {
            caught_exception: None,
            pc: 0,
            statements: Vec::new(),
            terminator: Terminator::Unreachable,
            handlers: Vec::new(),
        }]
        .into(),
        locals: IdVec::new(),
        parameters: Vec::new(),
        return_type: JvmType::Void,
        initialization_sites: Vec::new(),
    };
    let mut graph = Graph::build(&ir);
    graph.nodes = ops
        .into_iter()
        .map(|(ty, op)| Node {
            value: None,
            root: true,
            site: Site {
                block,
                statement: 0,
            },
            ty,
            op,
            guards: BTreeSet::new(),
            reachable: true,
        })
        .collect();
    graph
}

#[cfg(test)]
#[path = "../../tests/unit/semantic_ranges.rs"]
mod tests;
