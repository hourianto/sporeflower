//! Scheduled occurrences, rather than printable locals, are the semantic graph.
//! A local read is connected to the definitions which can reach that exact read.
//! Embedded writes and exceptional exits retain Java's evaluation order.
use super::Site;
use crate::classfile::JvmType;
use crate::index::IdVec;
use crate::ir::{
    self, BinaryOp, BlockId, CompareOp, Literal, LocalId, Member, MethodIr, ValueId, ValueKind,
    ValueUse,
    execution::{Boundary, Evaluator, Fold, Operation},
};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

pub(super) type NodeId = usize;

/// Allocation and delegation invoke the same constructor contract, but only an
/// allocation owns a new object. The distinction survives invocation lowering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Receiver {
    Static,
    Value(NodeId),
    New,
}
impl Receiver {
    pub fn value(self) -> Option<NodeId> {
        match self {
            Self::Value(node) => Some(node),
            Self::Static | Self::New => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum Op {
    Unknown,
    Parameter(usize),
    Literal(Literal),
    Read {
        local: LocalId,
        sources: BTreeSet<NodeId>,
    },
    Copy(NodeId),
    Select {
        left: NodeId,
        right: NodeId,
        comparison: CompareOp,
        yes: NodeId,
        no: NodeId,
    },
    Binary {
        op: BinaryOp,
        left: NodeId,
        right: NodeId,
    },
    Cast {
        value: NodeId,
        ty: JvmType,
    },
    Field(Member),
    ArrayLoad {
        array: NodeId,
        index: NodeId,
    },
    ArrayLength(NodeId),
    Call {
        method: Member,
        receiver: Receiver,
        args: Vec<NodeId>,
        offset: u32,
    },
    Array {
        elements: Vec<NodeId>,
    },
}

#[derive(Clone, Debug)]
pub(super) struct Node {
    pub value: Option<ValueId>,
    pub root: bool,
    pub site: Site,
    pub ty: JvmType,
    pub op: Op,
    pub guards: BTreeSet<Guard>,
    pub reachable: bool,
}

#[derive(Clone, Debug)]
pub(super) enum Sink {
    Field {
        field: Member,
        value: NodeId,
        pc: Option<u32>,
    },
    Store {
        array: NodeId,
        index: NodeId,
        value: NodeId,
    },
    Return(NodeId),
    Compare {
        left: NodeId,
        right: NodeId,
        op: CompareOp,
    },
    Switch {
        block: BlockId,
        value: NodeId,
    },
    Discard(NodeId),
    /// An operation observes a reference without forwarding its string meaning.
    Observe(NodeId),
}

#[derive(Debug)]
enum Event {
    Observe(NodeId),
    Write(LocalId, NodeId),
    Throw,
    /// A successful access checks its index after all operand effects and
    /// before later normal execution; its exceptional exit gets no bound.
    Index(NodeId),
    Choice {
        left: NodeId,
        right: NodeId,
        op: CompareOp,
        yes: Vec<Event>,
        no: Vec<Event>,
    },
}

#[derive(Debug)]
enum Exit {
    Goto(BlockId),
    Branch {
        left: NodeId,
        right: NodeId,
        op: CompareOp,
        yes: BlockId,
        no: BlockId,
    },
    Switch {
        value: NodeId,
        cases: Vec<(i32, BlockId)>,
        default: BlockId,
    },
    End,
}

#[derive(Debug)]
struct Block {
    events: Vec<Event>,
    exit: Exit,
}

/// A pure, captured value and an optional low-bit observation of that value.
/// Different evaluations of a field or call deliberately have distinct roots.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum KeyRoot {
    Value(NodeId),
    Local(LocalId, BTreeSet<NodeId>),
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct Key {
    pub root: KeyRoot,
    pub mask: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Guard {
    pub key: Key,
    pub relation: u8,
    pub number: i64,
    dependencies: BTreeSet<LocalId>,
}

#[derive(Clone, PartialEq, Eq)]
enum State {
    Definitions(IdVec<LocalId, BTreeSet<NodeId>>),
    Guards(BTreeSet<Guard>),
}

impl State {
    fn join(&mut self, other: &Self) -> bool {
        match (self, other) {
            (Self::Definitions(current), Self::Definitions(incoming)) => {
                let mut changed = false;
                for (local, definitions) in current.iter_enumerated_mut() {
                    let previous = definitions.len();
                    definitions.extend(&incoming[local]);
                    changed |= definitions.len() != previous;
                }
                changed
            }
            (Self::Guards(current), Self::Guards(incoming)) => {
                let previous = current.len();
                current.retain(|guard| incoming.contains(guard));
                current.len() != previous
            }
            _ => unreachable!("cannot join different semantic-flow passes"),
        }
    }
}

pub(super) struct Graph {
    pub nodes: Vec<Node>,
    pub sinks: Vec<Sink>,
    pub parameters: BTreeMap<usize, NodeId>,
}

impl Graph {
    pub fn build(ir: &MethodIr) -> Self {
        let mut graph = Self {
            nodes: Vec::new(),
            sinks: Vec::new(),
            parameters: BTreeMap::new(),
        };
        // Every uninitialized entry is explicit uncertainty; the empty set is
        // reserved for a not-yet-reached dataflow edge.
        let unknown = graph.add(
            None,
            true,
            Site {
                block: ir.entry,
                statement: 0,
            },
            JvmType::Int,
            Op::Unknown,
        );
        graph.nodes[unknown].reachable = true;
        let mut definitions: IdVec<LocalId, _> =
            vec![BTreeSet::from([unknown]); ir.locals.len()].into();
        for &value in &ir.parameters {
            let op = match ir.values[value].kind {
                ValueKind::Parameter {
                    index: Some(index), ..
                } => Op::Parameter(index),
                _ => Op::Unknown,
            };
            let node = graph.add(
                Some(value),
                true,
                Site {
                    block: ir.entry,
                    statement: 0,
                },
                ir.values[value].ty.clone(),
                op,
            );
            graph.nodes[node].reachable = true;
            if let Op::Parameter(index) = graph.nodes[node].op {
                graph.parameters.insert(index, node);
            }
            if let Some(local) = ir.values[value].local {
                definitions[local] = BTreeSet::from([node]);
            }
        }
        let blocks = Occurrences {
            ir,
            graph: &mut graph,
            site: Site {
                block: ir.entry,
                statement: 0,
            },
        }
        .build();
        graph.solve(ir, &blocks, State::Definitions(definitions));
        // Read sources are now frozen. Guard propagation uses those sources,
        // never the reaching-definition states from the first pass.
        graph.solve(ir, &blocks, State::Guards(BTreeSet::new()));
        graph
    }

    fn add(
        &mut self,
        value: Option<ValueId>,
        root: bool,
        site: Site,
        ty: JvmType,
        op: Op,
    ) -> NodeId {
        let node = self.nodes.len();
        self.nodes.push(Node {
            value,
            root,
            site,
            ty,
            op,
            guards: BTreeSet::new(),
            reachable: false,
        });
        node
    }

    fn solve(&mut self, ir: &MethodIr, blocks: &IdVec<BlockId, Block>, initial: State) {
        let mut entries: IdVec<BlockId, Option<State>> = vec![None; blocks.len()].into();
        entries[ir.entry] = Some(initial);
        let mut queue = VecDeque::from([ir.entry]);
        let mut queued: IdVec<BlockId, bool> = vec![false; blocks.len()].into();
        queued[ir.entry] = true;
        while let Some(block) = queue.pop_front() {
            queued[block] = false;
            let mut state = entries[block].as_ref().unwrap().clone();
            let mut exceptional = None;
            self.replay(
                &blocks[block].events,
                &mut state,
                (!ir.blocks[block].handlers.is_empty()).then_some(&mut exceptional),
            );
            let mut propagate = |target, state: Cow<'_, State>| {
                let changed = if let Some(current) = &mut entries[target] {
                    current.join(&state)
                } else {
                    entries[target] = Some(state.into_owned());
                    true
                };
                if changed && !queued[target] {
                    queued[target] = true;
                    queue.push_back(target);
                }
            };
            match &blocks[block].exit {
                Exit::Goto(target) => propagate(*target, Cow::Owned(state)),
                Exit::Branch {
                    left,
                    right,
                    op,
                    yes,
                    no,
                } => {
                    if matches!(state, State::Definitions(_)) {
                        // Only guards differ by edge; reaching definitions can
                        // join an existing successor without a temporary copy.
                        propagate(*yes, Cow::Borrowed(&state));
                        propagate(*no, Cow::Owned(state));
                    } else {
                        let mut other = state.clone();
                        self.assume(&mut state, *left, *right, *op);
                        self.assume(&mut other, *left, *right, op.inverted());
                        propagate(*yes, Cow::Owned(state));
                        propagate(*no, Cow::Owned(other));
                    }
                }
                Exit::Switch {
                    value,
                    cases,
                    default,
                } => {
                    for &(number, target) in cases {
                        let arm = if let State::Guards(guards) = &state {
                            let mut guards = guards.clone();
                            self.assume_number(
                                &mut guards,
                                *value,
                                i64::from(number),
                                CompareOp::Eq,
                            );
                            Cow::Owned(State::Guards(guards))
                        } else {
                            Cow::Borrowed(&state)
                        };
                        propagate(target, arm);
                    }
                    if let State::Guards(guards) = &mut state {
                        for &(number, _) in cases {
                            self.assume_number(guards, *value, i64::from(number), CompareOp::Ne);
                        }
                    }
                    propagate(*default, Cow::Owned(state));
                }
                Exit::End => {}
            }
            if let Some(state) = exceptional {
                for handler in crate::cfg::dispatch::effective(&ir.blocks[block].handlers) {
                    propagate(handler.target, Cow::Borrowed(&state));
                }
            }
        }
    }

    fn replay(
        &mut self,
        events: &[Event],
        state: &mut State,
        mut exceptional: Option<&mut Option<State>>,
    ) {
        for event in events {
            match event {
                Event::Observe(node) => {
                    self.nodes[*node].reachable = true;
                    match state {
                        State::Definitions(definitions) => {
                            if let Op::Read { local, sources } = &mut self.nodes[*node].op {
                                sources.extend(&definitions[*local]);
                            }
                        }
                        State::Guards(guards) => self.nodes[*node].guards = guards.clone(),
                    }
                }
                Event::Write(local, node) => match state {
                    State::Definitions(definitions) => {
                        definitions[*local] = BTreeSet::from([*node]);
                    }
                    State::Guards(guards) => {
                        guards.retain(|guard| !guard.dependencies.contains(local));
                    }
                },
                Event::Throw => {
                    if let Some(exceptional) = exceptional.as_deref_mut() {
                        if let Some(current) = exceptional {
                            current.join(state);
                        } else {
                            *exceptional = Some(state.clone());
                        }
                    }
                }
                Event::Index(index) => {
                    if let State::Guards(guards) = state {
                        let mut checked = BTreeSet::new();
                        self.assume_number(&mut checked, *index, 0, CompareOp::Ge);
                        self.assume_number(
                            &mut checked,
                            *index,
                            i64::from(i32::MAX),
                            CompareOp::Lt,
                        );
                        // The index's meaning describes a successfully loaded
                        // or stored element, including this access itself.
                        self.nodes[*index].guards.extend(checked.clone());
                        guards.extend(checked);
                    }
                }
                Event::Choice {
                    left,
                    right,
                    op,
                    yes,
                    no,
                } => {
                    let mut other = state.clone();
                    self.assume(state, *left, *right, *op);
                    self.assume(&mut other, *left, *right, op.inverted());
                    self.replay(yes, state, exceptional.as_deref_mut());
                    self.replay(no, &mut other, exceptional.as_deref_mut());
                    state.join(&other);
                }
            }
        }
    }

    fn assume(&self, state: &mut State, left: NodeId, right: NodeId, op: CompareOp) {
        let State::Guards(guards) = state else {
            return;
        };
        if let Some(number) = self.integer(right) {
            self.assume_number(guards, left, number, op);
        } else if let Some(number) = self.integer(left) {
            self.assume_number(guards, right, number, op.reversed());
        } else if !matches!(op, CompareOp::Eq | CompareOp::Ne) {
            // Even an unknown limit has JVM-width endpoints. In particular,
            // i < limit proves that incrementing an int i by one cannot wrap.
            // Narrow parameter descriptors are not runtime narrowing operations.
            for (value, peer, relation) in [(left, right, op), (right, left, op.reversed())] {
                let (min, max) = if self.nodes[peer].ty == JvmType::Long {
                    (i64::MIN, i64::MAX)
                } else if matches!(self.nodes[peer].op, Op::ArrayLength(_)) {
                    (0, i64::from(i32::MAX))
                } else {
                    (i64::from(i32::MIN), i64::from(i32::MAX))
                };
                let endpoint = match relation {
                    CompareOp::Lt | CompareOp::Le => max,
                    CompareOp::Gt | CompareOp::Ge => min,
                    _ => unreachable!(),
                };
                self.assume_number(guards, value, endpoint, relation);
            }
        }
    }

    fn assume_number(
        &self,
        guards: &mut BTreeSet<Guard>,
        node: NodeId,
        number: i64,
        op: CompareOp,
    ) {
        if let Some(key) = self.key(node) {
            let mut dependencies = BTreeSet::new();
            self.dependencies(node, &mut dependencies, &mut HashSet::new());
            guards.insert(Guard {
                key,
                relation: relation(op),
                number,
                dependencies,
            });
        }
    }

    fn dependencies(
        &self,
        node: NodeId,
        result: &mut BTreeSet<LocalId>,
        visiting: &mut HashSet<NodeId>,
    ) {
        if !visiting.insert(node) {
            return;
        }
        match &self.nodes[node].op {
            Op::Read { local, .. } => {
                result.insert(*local);
            }
            Op::Copy(value) | Op::Cast { value, .. } => self.dependencies(*value, result, visiting),
            Op::Binary { left, right, .. } => {
                self.dependencies(*left, result, visiting);
                self.dependencies(*right, result, visiting);
            }
            _ => {}
        }
    }

    pub fn integer(&self, node: NodeId) -> Option<i64> {
        self.integer_inner(node, &mut HashSet::new())
    }
    fn integer_inner(&self, node: NodeId, visiting: &mut HashSet<NodeId>) -> Option<i64> {
        if !visiting.insert(node) {
            return None;
        }
        let result = match &self.nodes[node].op {
            Op::Literal(Literal::Int(value)) => Some(i64::from(*value)),
            Op::Literal(Literal::Long(value)) => Some(*value),
            Op::Copy(value) => self.integer_inner(*value, visiting),
            Op::Read { sources, .. } => agreed(
                sources
                    .iter()
                    .map(|&source| self.integer_inner(source, visiting)),
            ),
            Op::Cast { value, ty } => {
                self.integer_inner(*value, visiting)
                    .and_then(|value| match ty {
                        JvmType::Byte => Some(i64::from(value as i8)),
                        JvmType::Short => Some(i64::from(value as i16)),
                        JvmType::Char => Some(i64::from(value as u16)),
                        JvmType::Int => Some(i64::from(value as i32)),
                        JvmType::Long => Some(value),
                        _ => None,
                    })
            }
            _ => None,
        };
        visiting.remove(&node);
        result
    }

    pub fn key(&self, node: NodeId) -> Option<Key> {
        self.key_inner(node, &mut HashSet::new())
    }
    fn key_inner(&self, node: NodeId, visiting: &mut HashSet<NodeId>) -> Option<Key> {
        if !visiting.insert(node) {
            return None;
        }
        let result = match &self.nodes[node].op {
            Op::Copy(value) => self.key_inner(*value, visiting),
            Op::Read { local, sources } => agreed(
                sources
                    .iter()
                    .map(|&source| self.key_inner(source, visiting)),
            )
            .or_else(|| {
                (!sources.is_empty()).then(|| Key {
                    root: KeyRoot::Local(*local, sources.clone()),
                    mask: None,
                })
            }),
            Op::Cast { value, ty } if *ty == JvmType::Long || self.nodes[*value].ty == *ty => {
                self.key_inner(*value, visiting)
            }
            Op::Binary {
                op: BinaryOp::And,
                left,
                right,
            } => {
                let pair = self
                    .integer(*right)
                    .map(|mask| (*left, mask))
                    .or_else(|| self.integer(*left).map(|mask| (*right, mask)));
                pair.and_then(|(value, mask)| {
                    self.key_inner(value, visiting).map(|mut key| {
                        key.mask = Some(key.mask.map_or(mask, |previous| previous & mask));
                        key
                    })
                })
            }
            Op::Literal(_) | Op::Unknown | Op::Select { .. } => None,
            _ => Some(Key {
                root: KeyRoot::Value(node),
                mask: None,
            }),
        };
        visiting.remove(&node);
        result
    }

    pub fn known_at(&self, at: NodeId, value: NodeId) -> Option<i64> {
        self.integer(value).or_else(|| {
            let key = self.key(value)?;
            self.nodes[at]
                .guards
                .iter()
                .find(|guard| guard.key == key && guard.relation == 0)
                .map(|guard| guard.number)
        })
    }
    pub fn excludes_at(&self, at: NodeId, value: NodeId, number: i64) -> bool {
        if let Some(value) = self.known_at(at, value) {
            return value != number;
        }
        self.key(value).is_some_and(|key| {
            self.nodes[at]
                .guards
                .iter()
                .any(|guard| guard.key == key && guard.relation == 1 && guard.number == number)
        })
    }
    pub fn masked_at(&self, at: NodeId, value: NodeId, mask: i64) -> Option<i64> {
        if let Some(value) = self.known_at(at, value) {
            return Some(value & mask);
        }
        let mut key = self.key(value)?;
        key.mask = Some(key.mask.map_or(mask, |previous| previous & mask));
        for guard in &self.nodes[at].guards {
            if guard.key != key {
                continue;
            }
            if guard.relation == 0 {
                return Some(guard.number);
            }
            // A one-bit observation has exactly two possible values. Java's
            // branch inversion commonly emits != 0 for the selected one.
            if guard.relation == 1 && (mask as u64).is_power_of_two() {
                if guard.number == 0 {
                    return Some(mask);
                }
                if guard.number == mask {
                    return Some(0);
                }
            }
        }
        None
    }
}

#[cfg(test)]
#[path = "../../tests/unit/semantic_flow.rs"]
mod tests;

fn agreed<T: PartialEq>(values: impl Iterator<Item = Option<T>>) -> Option<T> {
    let mut result = None;
    for value in values {
        let value = value?;
        if result.as_ref().is_some_and(|previous| *previous != value) {
            return None;
        }
        result = Some(value);
    }
    result
}
pub(super) fn relation(op: CompareOp) -> u8 {
    match op {
        CompareOp::Eq => 0,
        CompareOp::Ne => 1,
        CompareOp::Lt => 2,
        CompareOp::Le => 3,
        CompareOp::Gt => 4,
        CompareOp::Ge => 5,
    }
}

/// Semantic projection of the shared scheduled evaluator. The evaluator owns
/// storage boundaries and event order; this adapter owns occurrence identities,
/// semantic producers, and the consuming boundaries used by inference.
struct Occurrences<'a, 'g> {
    ir: &'a MethodIr,
    graph: &'g mut Graph,
    site: Site,
}

impl Occurrences<'_, '_> {
    fn build(self) -> IdVec<BlockId, Block> {
        let ir = self.ir;
        let mut evaluator = Evaluator::new(ir, self, false);
        let mut blocks = IdVec::with_capacity(ir.blocks.len());
        for (block_id, block) in ir.blocks.iter_enumerated() {
            evaluator.fold.site = Site {
                block: block_id,
                statement: 0,
            };
            let mut events = Vec::new();
            if let Some(value) = block.caught_exception {
                let node = evaluator.fold.observed(
                    Some(value),
                    true,
                    ir.values[value].ty.clone(),
                    Op::Unknown,
                    &mut events,
                );
                if let Some(local) = ir.values[value].local {
                    events.push(Event::Write(local, node));
                }
            }
            for (statement, action) in block.statements.iter().enumerate() {
                evaluator.fold.site.statement = statement;
                let inputs = evaluator.statement(action, &mut events);
                let graph = &mut evaluator.fold.graph;
                match action {
                    ir::Statement::Assign { .. } => {}
                    ir::Statement::Eval(_) => graph.sinks.push(Sink::Discard(inputs[0])),
                    ir::Statement::PutField {
                        field,
                        receiver,
                        pc,
                        ..
                    } => {
                        if receiver.is_some() {
                            graph.sinks.push(Sink::Observe(inputs[0]));
                        }
                        graph.sinks.push(Sink::Field {
                            field: field.clone(),
                            value: *inputs.last().unwrap(),
                            pc: *pc,
                        });
                    }
                    ir::Statement::ArrayStore { .. } => graph.sinks.push(Sink::Store {
                        array: inputs[0],
                        index: inputs[1],
                        value: inputs[2],
                    }),
                    ir::Statement::ConstructorCall { method, pc, .. } => {
                        let node = graph.add(
                            None,
                            true,
                            evaluator.fold.site,
                            JvmType::Void,
                            Op::Call {
                                method: method.clone(),
                                receiver: Receiver::Value(inputs[0]),
                                args: inputs[1..].to_vec(),
                                offset: *pc,
                            },
                        );
                        events.push(Event::Observe(node));
                    }
                    ir::Statement::Monitor { .. } => graph.sinks.push(Sink::Observe(inputs[0])),
                }
            }
            evaluator.fold.site.statement = block.statements.len();
            let inputs = evaluator.terminator(&block.terminator, &mut events);
            let graph = &mut evaluator.fold.graph;
            let exit = match &block.terminator {
                ir::Terminator::Goto(target) => Exit::Goto(*target),
                ir::Terminator::Branch {
                    condition,
                    then_target,
                    else_target,
                } => {
                    graph.sinks.push(Sink::Compare {
                        left: inputs[0],
                        right: inputs[1],
                        op: condition.op,
                    });
                    Exit::Branch {
                        left: inputs[0],
                        right: inputs[1],
                        op: condition.op,
                        yes: *then_target,
                        no: *else_target,
                    }
                }
                ir::Terminator::Switch { cases, default, .. } => {
                    graph.sinks.push(Sink::Switch {
                        block: block_id,
                        value: inputs[0],
                    });
                    Exit::Switch {
                        value: inputs[0],
                        cases: cases.clone(),
                        default: *default,
                    }
                }
                ir::Terminator::Return(_) => {
                    graph.sinks.extend(inputs.into_iter().map(Sink::Return));
                    Exit::End
                }
                ir::Terminator::Throw { .. } => {
                    graph.sinks.push(Sink::Observe(inputs[0]));
                    Exit::End
                }
                ir::Terminator::Unreachable => Exit::End,
            };
            blocks.push(Block { events, exit });
        }
        blocks
    }

    fn observed(
        &mut self,
        value: Option<ValueId>,
        root: bool,
        ty: JvmType,
        op: Op,
        events: &mut Vec<Event>,
    ) -> NodeId {
        let node = self.graph.add(value, root, self.site, ty, op);
        events.push(Event::Observe(node));
        node
    }
}

impl Fold for Occurrences<'_, '_> {
    type Value = NodeId;
    type Event = Event;

    fn value(&mut self, operation: Operation<'_, NodeId>, events: &mut Vec<Event>) -> NodeId {
        let previous = matches!(operation, Operation::Previous(..));
        let (usage, ty, op) = match operation {
            Operation::Read(usage, local) | Operation::Previous(usage, local) => (
                usage,
                if previous {
                    self.ir.values[usage.value()].ty.clone()
                } else {
                    self.ir.locals[local].ty.clone()
                },
                Op::Read {
                    local,
                    sources: BTreeSet::new(),
                },
            ),
            Operation::Cycle(usage) => {
                (usage, self.ir.values[usage.value()].ty.clone(), Op::Unknown)
            }
            Operation::Producer(usage, inputs) => return self.producer(usage, inputs, events),
            Operation::Target(usage, target, inputs) => {
                return self.target(usage, target, inputs, events);
            }
        };
        self.observed(
            Some(usage.value()),
            matches!(usage, ValueUse::Definition(_)),
            ty,
            op,
            events,
        )
    }

    fn effect(&mut self, boundary: Boundary<NodeId, Event>, events: &mut Vec<Event>) {
        events.push(match boundary {
            Boundary::Write(local, _, value, _) => Event::Write(local, value),
            Boundary::Throw => Event::Throw,
            Boundary::Index(value) => Event::Index(value),
            Boundary::Choice(op, [left, right], [yes, no]) => {
                self.graph.sinks.push(Sink::Compare { left, right, op });
                Event::Choice {
                    left,
                    right,
                    op,
                    yes,
                    no,
                }
            }
        });
    }
}

impl Occurrences<'_, '_> {
    fn producer(&mut self, usage: ValueUse, inputs: &[NodeId], events: &mut Vec<Event>) -> NodeId {
        let source = &self.ir.values[usage.value()];
        let op = match &source.kind {
            ValueKind::Literal(literal) => Op::Literal(literal.clone()),
            ValueKind::Parameter {
                index: Some(index), ..
            } => Op::Parameter(*index),
            ValueKind::Alias(_)
            | ValueKind::InlineAssign { .. }
            | ValueKind::LocalUpdate { .. } => Op::Copy(inputs[0]),
            ValueKind::Select { condition, .. } => Op::Select {
                left: inputs[0],
                right: inputs[1],
                comparison: condition.op,
                yes: inputs[2],
                no: inputs[3],
            },
            ValueKind::CompoundUpdate {
                op, operation_type, ..
            } => {
                let result = self.observed(
                    None,
                    false,
                    operation_type.clone(),
                    Op::Binary {
                        op: *op,
                        left: inputs[inputs.len() - 2],
                        right: inputs[inputs.len() - 1],
                    },
                    events,
                );
                Op::Cast {
                    value: result,
                    ty: source.ty.clone(),
                }
            }
            ValueKind::Binary { op, .. } => Op::Binary {
                op: *op,
                left: inputs[0],
                right: inputs[1],
            },
            ValueKind::Cast { ty, .. } => Op::Cast {
                value: inputs[0],
                ty: ty.clone(),
            },
            ValueKind::Field { field, .. }
            | ValueKind::Update {
                target: ir::UpdateTarget::Field { field, .. },
                ..
            } => {
                self.graph
                    .sinks
                    .extend(inputs.iter().copied().map(Sink::Observe));
                Op::Field(field.clone())
            }
            ValueKind::ArrayLoad { .. }
            | ValueKind::Update {
                target: ir::UpdateTarget::Array { .. },
                ..
            } => Op::ArrayLoad {
                array: inputs[0],
                index: inputs[1],
            },
            ValueKind::ArrayLength { .. } => Op::ArrayLength(inputs[0]),
            ValueKind::Call {
                method, receiver, ..
            } => Op::Call {
                method: method.clone(),
                receiver: if receiver.is_some() {
                    Receiver::Value(inputs[0])
                } else {
                    Receiver::Static
                },
                args: inputs[usize::from(receiver.is_some())..].to_vec(),
                offset: source.pc,
            },
            ValueKind::New {
                class, descriptor, ..
            } => Op::Call {
                method: Member {
                    owner: class.clone(),
                    name: "<init>".into(),
                    descriptor: descriptor.clone(),
                },
                receiver: Receiver::New,
                args: inputs.to_vec(),
                offset: source.pc,
            },
            ValueKind::ArrayInitializer { .. } => Op::Array {
                elements: inputs.to_vec(),
            },
            ValueKind::NewArray { .. } => Op::Array {
                elements: Vec::new(),
            },
            // Phi inputs describe incoming CFG edges, not evaluated children.
            ValueKind::Phi { .. } => Op::Unknown,
            _ => {
                self.graph
                    .sinks
                    .extend(inputs.iter().copied().map(Sink::Observe));
                Op::Unknown
            }
        };
        let node = self.graph.add(
            Some(usage.value()),
            matches!(usage, ValueUse::Definition(_)),
            self.site,
            source.ty.clone(),
            op,
        );
        if let ValueKind::CompoundUpdate { target, .. } = &source.kind {
            self.graph.sinks.push(match target {
                ir::UpdateTarget::Field { field, .. } => Sink::Field {
                    field: field.clone(),
                    value: node,
                    pc: Some(source.pc),
                },
                ir::UpdateTarget::Array { .. } => Sink::Store {
                    array: inputs[0],
                    index: inputs[1],
                    value: node,
                },
            });
        }
        events.push(Event::Observe(node));
        node
    }

    fn target(
        &mut self,
        usage: ValueUse,
        target: &ir::UpdateTarget,
        inputs: &[NodeId],
        events: &mut Vec<Event>,
    ) -> NodeId {
        let source = &self.ir.values[usage.value()];
        let op = match target {
            ir::UpdateTarget::Field { field, .. } => Op::Field(field.clone()),
            ir::UpdateTarget::Array { .. } => Op::ArrayLoad {
                array: inputs[0],
                index: inputs[1],
            },
        };
        let mut value = self.observed(None, false, source.ty.clone(), op, events);
        events.push(Event::Throw);
        if let ir::UpdateTarget::Array { .. } = target {
            events.push(Event::Index(inputs[1]));
        }
        if let ValueKind::CompoundUpdate { operation_type, .. } = &source.kind
            && source.ty != *operation_type
        {
            value = self.observed(
                None,
                false,
                operation_type.clone(),
                Op::Cast {
                    value,
                    ty: operation_type.clone(),
                },
                events,
            );
        }
        value
    }
}
