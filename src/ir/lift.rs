use super::*;
use crate::bytecode::{
    Operand,
    numeric::{BinaryOperation, Comparison, NumericOperation, Operands as NumericOperands},
    stack::{self, StackOperation},
};
use crate::classfile::{Code, Constant, MethodDescriptor};
use crate::hash::{HashMap, HashSet};
use crate::index::IdVec;
use anyhow::{Context, anyhow, bail, ensure};
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub(super) enum EventKind {
    Define(ValueId),
    /// A compound target is read before its RHS, then written at Define.
    ReadTarget(ValueId),
    /// Allocation begins before array-initializer operands are evaluated. The
    /// completed initializer keeps its ordinary Define event at the last store.
    Allocate(ValueId),
    Action(Statement),
    ThrowPoint,
}

#[derive(Clone, Debug)]
pub(super) struct Event {
    pub pc: u32,
    pub kind: EventKind,
}

#[derive(Clone, Debug)]
pub(super) struct RawIr {
    pub ir: MethodIr,
    pub events: IdVec<BlockId, Vec<Event>>,
    statement_pcs: IdVec<BlockId, Vec<u32>>,
    /// Original stores supply source-slot provenance even without debug names.
    /// Reads are retained only when local-variable metadata can name them.
    pub local_accesses: Vec<LocalAccess>,
    /// Definitions owned by each lazy Select arm. Their events retain bytecode
    /// identity, but are evaluated inside the arm, never as block statements.
    pub conditional_regions: HashMap<ValueId, [Vec<ValueId>; 2]>,
}

impl RawIr {
    pub fn new(ir: MethodIr) -> Self {
        Self {
            events: vec![vec![]; ir.blocks.len()].into(),
            statement_pcs: ir
                .blocks
                .iter()
                .map(|block| vec![u32::MAX; block.statements.len()])
                .collect(),
            ir,
            local_accesses: vec![],
            conditional_regions: HashMap::default(),
        }
    }

    pub fn statement_pcs(&self, block: BlockId) -> &[u32] {
        &self.statement_pcs[block]
    }

    /// Structural statement edits own their origins until scheduling finishes.
    /// Rewriting an operand or destination in place preserves its existing PC.
    pub fn set_statements(&mut self, block: BlockId, statements: Vec<(Statement, u32)>) {
        (self.ir.blocks[block].statements, self.statement_pcs[block]) =
            statements.into_iter().unzip();
    }

    pub fn extend_statements(
        &mut self,
        block: BlockId,
        statements: impl IntoIterator<Item = (Statement, u32)>,
    ) {
        for (statement, pc) in statements {
            self.ir.blocks[block].statements.push(statement);
            self.statement_pcs[block].push(pc);
        }
    }

    pub fn retain_statements(&mut self, block: BlockId, mut keep: impl FnMut(usize) -> bool) {
        let pcs = &mut self.statement_pcs[block];
        let mut source = 0;
        let mut target = 0;
        self.ir.blocks[block].statements.retain(|_| {
            let retained = keep(source);
            if retained {
                pcs[target] = pcs[source];
                target += 1;
            }
            source += 1;
            retained
        });
        pcs.truncate(target);
    }

    pub fn remove_statement(&mut self, block: BlockId, index: usize) -> (Statement, u32) {
        (
            self.ir.blocks[block].statements.remove(index),
            self.statement_pcs[block].remove(index),
        )
    }

    pub fn push_edge(&mut self, source: BlockId, target: BlockId) -> BlockId {
        let edge = self.ir.blocks.next_id();
        self.ir.blocks.push(IrBlock {
            caught_exception: None,
            pc: self.ir.blocks[source].pc,
            statements: vec![],
            terminator: Terminator::Goto(target),
            handlers: vec![],
        });
        self.statement_pcs.push(vec![]);
        edge
    }

    /// Expose a nonthrowing suffix after a monitor. Its statements execute on
    /// the normal edge, outside the monitor operation's exception boundary.
    pub fn split_nonthrowing_suffix(&mut self, block: BlockId, at: usize) {
        let edge = self.push_edge(block, block);
        self.ir.blocks[edge].statements = self.ir.blocks[block].statements.split_off(at);
        self.statement_pcs[edge] = self.statement_pcs[block].split_off(at);
        self.ir.blocks[edge].terminator = std::mem::replace(
            &mut self.ir.blocks[block].terminator,
            Terminator::Goto(edge),
        );
    }

    pub fn assert_statement_origins(&self) {
        debug_assert_eq!(self.ir.blocks.len(), self.statement_pcs.len());
        debug_assert!(
            self.ir
                .blocks
                .iter_enumerated()
                .all(|(block, body)| { body.statements.len() == self.statement_pcs[block].len() })
        );
    }
}

#[derive(Clone, Debug)]
pub(super) struct LocalAccess {
    pub value: ValueId,
    /// The frame value overwritten by this store. Independent arithmetic
    /// assigned to a fresh slot is not a mutation of its input binding.
    pub replaced: Option<ValueId>,
    pub slot: u16,
    pub pc: u32,
    pub next_pc: u32,
    pub store: bool,
}

#[derive(Clone, Debug)]
struct Frame {
    locals: Vec<Option<ValueId>>,
    stack: Vec<ValueId>,
}

#[derive(Default)]
struct BlockState {
    entry: Option<Frame>,
    processed: bool,
}

struct Lifter<'a> {
    class: &'a ClassFile,
    code: &'a Code,
    instructions: &'a [Instruction],
    cfg: &'a ControlFlowGraph,
    raw: RawIr,
    states: IdVec<BlockId, BlockState>,
    queue: VecDeque<BlockId>,
    layout: super::liveness::LocalLayout,
    live: super::liveness::LiveLocals,
}

pub(super) fn lift(
    class: &ClassFile,
    method: &Method,
    instructions: &[Instruction],
    cfg: &ControlFlowGraph,
) -> Result<RawIr> {
    let code = method
        .code
        .as_ref()
        .ok_or_else(|| anyhow!("method has no code"))?;
    let descriptor = MethodDescriptor::parse(&method.descriptor)?;
    let mut parameter_slots = vec![];
    let mut parameter_slot = 0;
    if method.access_flags & 0x0008 == 0 {
        parameter_slots.push(0);
        parameter_slot = 1;
    }
    for parameter in &descriptor.parameters {
        parameter_slots.push(parameter_slot);
        parameter_slot += parameter.slots();
    }
    let layout =
        super::liveness::LocalLayout::new(instructions, &parameter_slots, code.max_locals)?;
    let live = super::liveness::analyze(instructions, cfg, &layout);
    let local_count = layout.slots.len();
    ensure!(!cfg.blocks.is_empty(), "empty method body");
    let prologue = cfg.blocks.next_id();
    let mut blocks: IdVec<BlockId, _> = cfg
        .blocks
        .iter()
        .map(|block| IrBlock {
            caught_exception: None,
            pc: instructions[block.start].pc,
            statements: Vec::new(),
            terminator: Terminator::Unreachable,
            handlers: block.handlers.clone(),
        })
        .collect();
    blocks.push(IrBlock {
        caught_exception: None,
        pc: 0,
        statements: vec![],
        terminator: Terminator::Goto(cfg.entry),
        handlers: vec![],
    });
    let mut lifter = Lifter {
        class,
        code,
        instructions,
        cfg,
        raw: RawIr::new(MethodIr {
            entry: prologue,
            values: IdVec::new(),
            blocks,
            locals: IdVec::new(),
            parameters: vec![],
            return_type: descriptor.return_type,
            initialization_sites: vec![],
        }),
        states: cfg.blocks.ids().map(|_| BlockState::default()).collect(),
        queue: VecDeque::new(),
        layout,
        live,
    };
    let mut frame = Frame {
        locals: vec![None; local_count],
        stack: vec![],
    };
    let mut slot = 0;
    if method.access_flags & 0x0008 == 0 {
        let value = lifter.value(
            prologue,
            0,
            JvmType::Object(class.name.clone()),
            ValueKind::Parameter {
                slot: 0,
                index: None,
            },
        );
        lifter.raw.ir.parameters.push(value);
        lifter.store(&mut frame, 0, value)?;
        slot = 1;
    }
    for (index, ty) in descriptor.parameters.into_iter().enumerate() {
        let width = ty.slots();
        let value = lifter.value(
            prologue,
            0,
            ty,
            ValueKind::Parameter {
                slot,
                index: Some(index),
            },
        );
        lifter.raw.ir.parameters.push(value);
        lifter.store(&mut frame, slot, value)?;
        slot += width;
    }
    lifter.incoming(cfg.entry, prologue, None, &frame.locals, &frame.stack)?;
    while let Some(block) = lifter.queue.pop_front() {
        if lifter.states[block].processed {
            continue;
        }
        lifter.states[block].processed = true;
        lifter
            .process(block)
            .with_context(|| format!("{}{} at block {}", method.name, method.descriptor, block))?;
    }
    canonicalize(&mut lifter.raw);
    Ok(lifter.raw)
}

impl Lifter<'_> {
    fn value(&mut self, block: BlockId, pc: u32, ty: JvmType, kind: ValueKind) -> ValueId {
        let value = self.raw.ir.values.next_id();
        self.raw.ir.values.push(Value {
            local: None,
            kind,
            ty,
            pc,
            block,
        });
        value
    }

    fn define(&mut self, block: BlockId, pc: u32, ty: JvmType, kind: ValueKind) -> ValueId {
        let value = self.value(block, pc, ty, kind);
        self.raw.events[block].push(Event {
            pc,
            kind: EventKind::Define(value),
        });
        value
    }

    fn action(&mut self, block: BlockId, pc: u32, action: Statement) {
        self.raw.events[block].push(Event {
            pc,
            kind: EventKind::Action(action),
        });
    }

    fn phi(
        &mut self,
        block: BlockId,
        source: ValueId,
        slot: Option<u16>,
        pred: BlockId,
        throw_pc: Option<u32>,
    ) -> ValueId {
        self.value(
            block,
            self.raw.ir.blocks[block].pc,
            self.raw.ir.values[source].ty.clone(),
            ValueKind::Phi {
                slot,
                inputs: vec![PhiInput {
                    block: pred,
                    throw_pc,
                    value: source,
                }],
            },
        )
    }

    fn incoming(
        &mut self,
        block: BlockId,
        pred: BlockId,
        throw_pc: Option<u32>,
        locals: &[Option<ValueId>],
        stack: &[ValueId],
    ) -> Result<()> {
        if let Some(entry) = self.states[block].entry.as_ref() {
            ensure!(
                entry.stack.len() == stack.len(),
                "incompatible operand stack heights at block {block}"
            );
            for (&phi, &source) in entry.locals.iter().zip(locals) {
                if let (Some(phi), Some(source)) = (phi, source)
                    && let ValueKind::Phi { inputs, .. } = &mut self.raw.ir.values[phi].kind
                {
                    inputs.push(PhiInput {
                        block: pred,
                        throw_pc,
                        value: source,
                    });
                }
            }
            for (&phi, &source) in entry.stack.iter().zip(stack) {
                ensure!(
                    self.raw.ir.values[phi].ty.slots() == self.raw.ir.values[source].ty.slots(),
                    "incompatible operand stack categories at block {block}"
                );
                if let ValueKind::Phi { inputs, .. } = &mut self.raw.ir.values[phi].kind {
                    inputs.push(PhiInput {
                        block: pred,
                        throw_pc,
                        value: source,
                    });
                }
            }
        } else {
            let mut entry = Frame {
                locals: vec![None; locals.len()],
                stack: Vec::with_capacity(stack.len()),
            };
            for (slot, &source) in locals.iter().enumerate() {
                if self.live.contains(block, slot)
                    && let Some(source) = source
                {
                    entry.locals[slot] = Some(self.phi(
                        block,
                        source,
                        Some(self.layout.slots[slot]),
                        pred,
                        throw_pc,
                    ));
                }
            }
            for &source in stack {
                // A handler PC may also have ordinary incoming control flow.
                // Keep its stack join until all edges are known; a catch-only
                // phi later canonicalizes to the one CaughtException value.
                entry
                    .stack
                    .push(self.phi(block, source, None, pred, throw_pc));
            }
            self.states[block].entry = Some(entry);
            self.queue.push_back(block);
        }
        Ok(())
    }

    fn pop(&self, frame: &mut Frame) -> Result<ValueId> {
        stack::pop(&mut frame.stack)
    }

    fn pop1(&self, frame: &mut Frame) -> Result<ValueId> {
        stack::pop_one(&mut frame.stack, |&value| self.category(value))
    }

    fn category(&self, value: ValueId) -> u16 {
        self.raw.ir.values[value].ty.slots()
    }

    fn store(&self, frame: &mut Frame, slot: u16, value: ValueId) -> Result<()> {
        let width = self.category(value);
        ensure!(
            self.layout.valid_width(slot, width),
            "local slot {} exceeds max_locals",
            slot
        );
        let index = self.layout.index(slot)?;
        if slot > 0
            && let Some(previous) = self.layout.optional(slot - 1)
            && frame.locals[previous].is_some_and(|value| self.category(value) == 2)
        {
            frame.locals[previous] = None;
        }
        frame.locals[index] = Some(value);
        if width == 2
            && let Some(next) = self.layout.optional(slot + 1)
        {
            frame.locals[next] = None;
        }
        Ok(())
    }

    fn local(&self, frame: &Frame, slot: u16) -> Result<ValueId> {
        frame.locals[self.layout.index(slot)?]
            .ok_or_else(|| anyhow!("read of undefined local {slot}"))
    }

    fn target(&self, pc: u32) -> Result<BlockId> {
        self.cfg
            .block_at_pc(self.instructions, pc)
            .ok_or_else(|| anyhow!("invalid branch target {pc}"))
    }

    fn member(&self, index: u16) -> Result<Member> {
        let member = self.class.constant_pool.member_ref(index)?;
        Ok(Member {
            owner: member.owner.to_owned(),
            name: member.name.to_owned(),
            descriptor: member.descriptor.to_owned(),
        })
    }

    fn literal(&mut self, block: BlockId, pc: u32, literal: Literal) -> ValueId {
        let ty = match &literal {
            Literal::Null => JvmType::Object("java/lang/Object".into()),
            Literal::Int(_) => JvmType::Int,
            Literal::Long(_) => JvmType::Long,
            Literal::Float(_) => JvmType::Float,
            Literal::Double(_) => JvmType::Double,
            Literal::String(_) => JvmType::Object("java/lang/String".into()),
            Literal::Class(_) => JvmType::Object("java/lang/Class".into()),
        };
        self.define(block, pc, ty, ValueKind::Literal(literal))
    }

    fn exception_edges(&mut self, block: BlockId, pc: u32, frame: &Frame) -> Result<()> {
        let handlers = &self.cfg.blocks[block].handlers;
        if handlers.is_empty() {
            return Ok(());
        }
        self.raw.events[block].push(Event {
            pc,
            kind: EventKind::ThrowPoint,
        });
        for handler in crate::cfg::dispatch::effective(handlers) {
            let caught = if let Some(value) = self.raw.ir.blocks[handler.target].caught_exception {
                value
            } else {
                let ty = JvmType::Object(
                    handler
                        .catch_type
                        .clone()
                        .unwrap_or_else(|| "java/lang/Throwable".into()),
                );
                let value = self.value(
                    handler.target,
                    self.raw.ir.blocks[handler.target].pc,
                    ty,
                    ValueKind::CaughtException {
                        handler: handler.target,
                    },
                );
                self.raw.ir.blocks[handler.target].caught_exception = Some(value);
                value
            };
            let catch_type = JvmType::Object(
                handler
                    .catch_type
                    .clone()
                    .unwrap_or_else(|| "java/lang/Throwable".into()),
            );
            if self.raw.ir.values[caught].ty != catch_type {
                // Every incoming exception is a Throwable, including catch-all
                // entries. An ordinary reference join would lose that guarantee
                // at Object, while ignoring catch-all would retain an unsafe
                // narrower type from whichever table entry was visited first.
                self.raw.ir.values[caught].ty = JvmType::Object("java/lang/Throwable".into());
            }
            self.incoming(handler.target, block, Some(pc), &frame.locals, &[caught])?;
        }
        Ok(())
    }

    fn allocation(&self, value: ValueId) -> Option<(ValueId, String)> {
        let mut visiting = HashSet::default();
        self.allocation_inner(value, &mut visiting)
    }

    fn allocation_inner(
        &self,
        value: ValueId,
        visiting: &mut HashSet<ValueId>,
    ) -> Option<(ValueId, String)> {
        if !visiting.insert(value) {
            return None;
        }
        let result = match &self.raw.ir.values[value].kind {
            ValueKind::Allocation { class } => Some((value, class.clone())),
            ValueKind::Phi { inputs, .. } => {
                let mut origin = None;
                for input in inputs {
                    if input.value == value {
                        continue;
                    }
                    if let Some(found) = self.allocation_inner(input.value, visiting) {
                        if origin.as_ref().is_some_and(|old| old != &found) {
                            return None;
                        }
                        origin = Some(found);
                    } else if !visiting.contains(&input.value) {
                        return None;
                    }
                }
                origin
            }
            _ => None,
        };
        visiting.remove(&value);
        result
    }

    fn process(&mut self, block: BlockId) -> Result<()> {
        let mut frame = self.states[block].entry.clone().unwrap();
        let span = self.cfg.blocks[block].start..self.cfg.blocks[block].end;
        let mut ended = false;
        for index in span {
            let instruction = &self.instructions[index];
            let pc = instruction.pc;
            let next_pc = self
                .instructions
                .get(index + 1)
                .map_or(self.code.bytes.len() as u32, |next| next.pc);
            let opcode = instruction.opcode;
            let resolves_constant = if matches!(opcode, 0x12..=0x14) {
                matches!(instruction.operand, Operand::Constant(index)
                    if matches!(self.class.constant_pool.get(index)?, Constant::String(_) | Constant::Class(_)))
            } else {
                true
            };
            if instruction.can_throw() && resolves_constant {
                self.exception_edges(block, pc, &frame)?;
            }
            let operand = &instruction.operand;
            let constant_index = || instruction.constant_index();
            let local_index = || instruction.local_index();
            match opcode {
                0x00 => {}
                0x01 => {
                    let value = self.literal(block, pc, Literal::Null);
                    frame.stack.push(value);
                }
                0x02..=0x08 => {
                    let value = self.literal(block, pc, Literal::Int(opcode as i32 - 3));
                    frame.stack.push(value);
                }
                0x09..=0x0a => {
                    let value = self.literal(block, pc, Literal::Long((opcode - 0x09) as i64));
                    frame.stack.push(value);
                }
                0x0b..=0x0d => {
                    let value = self.literal(block, pc, Literal::Float((opcode - 0x0b) as f32));
                    frame.stack.push(value);
                }
                0x0e..=0x0f => {
                    let value = self.literal(block, pc, Literal::Double((opcode - 0x0e) as f64));
                    frame.stack.push(value);
                }
                0x10 | 0x11 => {
                    let Operand::Int(number) = operand else {
                        bail!("missing immediate at {pc}")
                    };
                    let value = self.literal(block, pc, Literal::Int(*number));
                    frame.stack.push(value);
                }
                0x12..=0x14 => {
                    let index = constant_index()?;
                    let literal = match self.class.constant_pool.get(index)? {
                        Constant::Integer(value) => Literal::Int(*value),
                        Constant::Long(value) => Literal::Long(*value),
                        Constant::Float(value) => Literal::Float(*value),
                        Constant::Double(value) => Literal::Double(*value),
                        Constant::String(_) => {
                            Literal::String(self.class.constant_pool.string_utf16(index)?)
                        }
                        Constant::Class(_) => {
                            Literal::Class(class_type(self.class.constant_pool.class_name(index)?)?)
                        }
                        _ => bail!("unsupported ldc constant at {pc}"),
                    };
                    let value = self.literal(block, pc, literal);
                    frame.stack.push(value);
                }
                0x15..=0x19 => {
                    let slot = local_index()?;
                    let value = self.local(&frame, slot)?;
                    if !self.code.local_variables.is_empty() {
                        self.raw.local_accesses.push(LocalAccess {
                            value,
                            replaced: None,
                            slot,
                            pc,
                            next_pc,
                            store: false,
                        });
                    }
                    frame.stack.push(value);
                }
                0x2e..=0x35 => {
                    let index = self.pop1(&mut frame)?;
                    let array = self.pop1(&mut frame)?;
                    let element_type = array_element(opcode - 0x2e, &self.raw.ir.values[array].ty);
                    let value = self.define(
                        block,
                        pc,
                        element_type.clone(),
                        ValueKind::ArrayLoad {
                            array,
                            index,
                            element_type,
                        },
                    );
                    frame.stack.push(value);
                }
                0x36..=0x3a => {
                    let value = self.pop(&mut frame)?;
                    let slot = local_index()?;
                    let replaced = frame.locals[self.layout.index(slot)?];
                    self.store(&mut frame, slot, value)?;
                    self.raw.local_accesses.push(LocalAccess {
                        value,
                        replaced,
                        slot,
                        pc,
                        next_pc,
                        store: true,
                    });
                }
                0x4f..=0x56 => {
                    let value = self.pop(&mut frame)?;
                    let index = self.pop1(&mut frame)?;
                    let array = self.pop1(&mut frame)?;
                    let element_type = array_element(opcode - 0x4f, &self.raw.ir.values[array].ty);
                    self.action(
                        block,
                        pc,
                        Statement::ArrayStore {
                            array,
                            index,
                            value,
                            element_type,
                        },
                    );
                }
                0x57..=0x5f => StackOperation::decode(opcode)
                    .unwrap()
                    .apply(&mut frame.stack, |&value| self.category(value))?,
                0x60..=0x83 | 0x85..=0x98 => {
                    let operation = NumericOperation::decode(opcode).unwrap();
                    let ty = operation.result_type();
                    let operands = operation.operands(&mut frame.stack)?;
                    let kind = match (operation, operands) {
                        (
                            NumericOperation::Binary { operation, .. },
                            NumericOperands::Binary { left, right },
                        ) => ValueKind::Binary {
                            op: binary_op(operation),
                            left,
                            right,
                        },
                        (NumericOperation::Negate(_), NumericOperands::Unary(value)) => {
                            ValueKind::Unary {
                                op: UnaryOp::Neg,
                                value,
                            }
                        }
                        (NumericOperation::Convert(_), NumericOperands::Unary(value)) => {
                            ValueKind::Cast {
                                value,
                                ty: ty.clone(),
                            }
                        }
                        (
                            NumericOperation::Compare(kind),
                            NumericOperands::Binary { left, right },
                        ) => ValueKind::Compare {
                            kind: comparison(kind),
                            left,
                            right,
                        },
                        _ => unreachable!("numeric operands match the decoded family"),
                    };
                    let value = self.define(block, pc, ty, kind);
                    frame.stack.push(value);
                }
                0x84 => {
                    let Operand::Increment { local, amount } = operand else {
                        bail!("invalid iinc at {pc}")
                    };
                    let left = self.local(&frame, *local)?;
                    if !self.code.local_variables.is_empty() {
                        self.raw.local_accesses.push(LocalAccess {
                            value: left,
                            replaced: None,
                            slot: *local,
                            pc,
                            next_pc,
                            store: false,
                        });
                    }
                    let right = self.literal(block, pc, Literal::Int(*amount as i32));
                    let value = self.define(
                        block,
                        pc,
                        JvmType::Int,
                        ValueKind::Binary {
                            op: BinaryOp::Add,
                            left,
                            right,
                        },
                    );
                    self.store(&mut frame, *local, value)?;
                    self.raw.local_accesses.push(LocalAccess {
                        value,
                        replaced: Some(left),
                        slot: *local,
                        pc,
                        next_pc,
                        store: true,
                    });
                }
                0x99..=0xa6 | 0xc6 | 0xc7 => {
                    let (left, right, op) = match opcode {
                        0x99..=0x9e => {
                            let left = self.pop1(&mut frame)?;
                            let right = self.literal(block, pc, Literal::Int(0));
                            (
                                left,
                                right,
                                [
                                    CompareOp::Eq,
                                    CompareOp::Ne,
                                    CompareOp::Lt,
                                    CompareOp::Ge,
                                    CompareOp::Gt,
                                    CompareOp::Le,
                                ][(opcode - 0x99) as usize],
                            )
                        }
                        0x9f..=0xa6 => {
                            let right = self.pop1(&mut frame)?;
                            let left = self.pop1(&mut frame)?;
                            let op = [
                                CompareOp::Eq,
                                CompareOp::Ne,
                                CompareOp::Lt,
                                CompareOp::Ge,
                                CompareOp::Gt,
                                CompareOp::Le,
                                CompareOp::Eq,
                                CompareOp::Ne,
                            ][(opcode - 0x9f) as usize];
                            (left, right, op)
                        }
                        _ => {
                            let left = self.pop1(&mut frame)?;
                            let right = self.literal(block, pc, Literal::Null);
                            (
                                left,
                                right,
                                if opcode == 0xc6 {
                                    CompareOp::Eq
                                } else {
                                    CompareOp::Ne
                                },
                            )
                        }
                    };
                    let Operand::Branch(target) = operand else {
                        bail!("missing branch operand")
                    };
                    let then_target = self.target(*target)?;
                    let else_target = self
                        .cfg
                        .instruction_blocks
                        .get(index + 1)
                        .copied()
                        .ok_or_else(|| anyhow!("conditional falls off method"))?;
                    self.raw.ir.blocks[block].terminator = Terminator::Branch {
                        condition: Condition { op, left, right },
                        then_target,
                        else_target,
                    };
                    ended = true;
                }
                0xa7 => {
                    let Operand::Branch(target) = operand else {
                        bail!("missing goto operand")
                    };
                    self.raw.ir.blocks[block].terminator = Terminator::Goto(self.target(*target)?);
                    ended = true;
                }
                0xa8 | 0xa9 => bail!("legacy jsr/ret must be normalized before SSA lifting"),
                0xaa | 0xab => {
                    let value = self.pop1(&mut frame)?;
                    let Operand::Switch { default, cases } = operand else {
                        bail!("invalid switch operand")
                    };
                    let cases = cases
                        .iter()
                        .map(|&(key, pc)| Ok((key, self.target(pc)?)))
                        .collect::<Result<Vec<_>>>()?;
                    self.raw.ir.blocks[block].terminator = Terminator::Switch {
                        value,
                        cases,
                        default: self.target(*default)?,
                    };
                    ended = true;
                }
                0xac..=0xb0 => {
                    let value = self.pop(&mut frame)?;
                    self.raw.ir.blocks[block].terminator = Terminator::Return(Some(value));
                    ended = true;
                }
                0xb1 => {
                    self.raw.ir.blocks[block].terminator = Terminator::Return(None);
                    ended = true;
                }
                0xb2 | 0xb4 => {
                    let field = self.member(constant_index()?)?;
                    let ty = JvmType::parse(&field.descriptor)?;
                    let receiver = if opcode == 0xb4 {
                        Some(self.pop1(&mut frame)?)
                    } else {
                        None
                    };
                    let value = self.define(block, pc, ty, ValueKind::Field { field, receiver });
                    frame.stack.push(value);
                }
                0xb3 | 0xb5 => {
                    let field = self.member(constant_index()?)?;
                    let value = self.pop(&mut frame)?;
                    let receiver = if opcode == 0xb5 {
                        Some(self.pop1(&mut frame)?)
                    } else {
                        None
                    };
                    self.action(
                        block,
                        pc,
                        Statement::PutField {
                            pc: Some(pc),
                            field,
                            receiver,
                            value,
                        },
                    );
                }
                0xb6..=0xb9 => {
                    let method = self.member(constant_index()?)?;
                    let descriptor = MethodDescriptor::parse(&method.descriptor)?;
                    let args =
                        stack::arguments(&mut frame.stack, &descriptor.parameters, |&value| {
                            self.category(value)
                        })
                        .with_context(|| format!("invocation arguments at {pc}"))?;
                    let receiver = if opcode == 0xb8 {
                        None
                    } else {
                        Some(self.pop1(&mut frame)?)
                    };
                    if method.name == "<init>" {
                        let receiver =
                            receiver.ok_or_else(|| anyhow!("static constructor invocation"))?;
                        if let Some((allocation, class)) = self.allocation(receiver) {
                            let allocation_pc = self.raw.ir.values[allocation].pc;
                            let value = self.define(
                                block,
                                pc,
                                JvmType::Object(class.clone()),
                                ValueKind::New {
                                    class,
                                    descriptor: method.descriptor,
                                    args,
                                    allocation_pc,
                                },
                            );
                            let replacements: Vec<_> = frame
                                .locals
                                .iter()
                                .flatten()
                                .chain(frame.stack.iter())
                                .copied()
                                .filter(|&alias| {
                                    self.allocation(alias)
                                        .is_some_and(|(origin, _)| origin == allocation)
                                })
                                .collect();
                            for alias in frame
                                .locals
                                .iter_mut()
                                .flatten()
                                .chain(frame.stack.iter_mut())
                            {
                                if replacements.contains(alias) {
                                    *alias = value;
                                }
                            }
                        } else {
                            self.action(
                                block,
                                pc,
                                Statement::ConstructorCall {
                                    pc,
                                    method,
                                    receiver,
                                    args,
                                },
                            );
                        }
                    } else {
                        let kind = match opcode {
                            0xb6 => InvokeKind::Virtual,
                            0xb7 => InvokeKind::Special,
                            0xb8 => InvokeKind::Static,
                            _ => InvokeKind::Interface,
                        };
                        let value = self.define(
                            block,
                            pc,
                            descriptor.return_type.clone(),
                            ValueKind::Call {
                                kind,
                                method,
                                receiver,
                                args,
                            },
                        );
                        if descriptor.return_type != JvmType::Void {
                            frame.stack.push(value);
                        }
                    }
                }
                0xba => bail!("invokedynamic is outside J2ME bytecode"),
                0xbb => {
                    let class = self
                        .class
                        .constant_pool
                        .class_name(constant_index()?)?
                        .to_owned();
                    let value = self.define(
                        block,
                        pc,
                        JvmType::Object(class.clone()),
                        ValueKind::Allocation { class },
                    );
                    frame.stack.push(value);
                }
                0xbc | 0xbd => {
                    let count = self.pop1(&mut frame)?;
                    let element_type = if opcode == 0xbd {
                        class_type(self.class.constant_pool.class_name(constant_index()?)?)?
                    } else {
                        instruction.newarray_type()?
                    };
                    let value = self.define(
                        block,
                        pc,
                        JvmType::Array(Box::new(element_type.clone())),
                        ValueKind::NewArray {
                            element_type,
                            dimensions: vec![count],
                        },
                    );
                    frame.stack.push(value);
                }
                0xbe => {
                    let array = self.pop1(&mut frame)?;
                    let value =
                        self.define(block, pc, JvmType::Int, ValueKind::ArrayLength { array });
                    frame.stack.push(value);
                }
                0xbf => {
                    let value = self.pop1(&mut frame)?;
                    self.raw.ir.blocks[block].terminator = Terminator::Throw { value, pc };
                    ended = true;
                }
                0xc0 | 0xc1 => {
                    let value = self.pop1(&mut frame)?;
                    let ty = class_type(self.class.constant_pool.class_name(constant_index()?)?)?;
                    let (result_type, kind) = if opcode == 0xc0 {
                        (ty.clone(), ValueKind::Cast { value, ty })
                    } else {
                        (JvmType::Boolean, ValueKind::InstanceOf { value, ty })
                    };
                    let value = self.define(block, pc, result_type, kind);
                    frame.stack.push(value);
                }
                0xc2 | 0xc3 => {
                    let value = self.pop1(&mut frame)?;
                    self.action(
                        block,
                        pc,
                        Statement::Monitor {
                            value,
                            enter: opcode == 0xc2,
                        },
                    );
                }
                0xc5 => {
                    let Operand::MultiArray { index, dimensions } = operand else {
                        bail!("invalid multianewarray operand")
                    };
                    let ty = class_type(self.class.constant_pool.class_name(*index)?)?;
                    let mut element_type = ty.clone();
                    let mut sizes = vec![];
                    for _ in 0..*dimensions {
                        sizes.push(self.pop1(&mut frame)?);
                        let JvmType::Array(element) = element_type else {
                            bail!("too many multianewarray dimensions")
                        };
                        element_type = *element;
                    }
                    sizes.reverse();
                    let value = self.define(
                        block,
                        pc,
                        ty,
                        ValueKind::NewArray {
                            element_type,
                            dimensions: sizes,
                        },
                    );
                    frame.stack.push(value);
                }
                _ => bail!("unsupported JVM opcode 0x{opcode:02x} at {pc}"),
            }
            let stack_slots: u16 = frame.stack.iter().map(|&value| self.category(value)).sum();
            ensure!(
                stack_slots <= self.code.max_stack,
                "operand stack exceeds max_stack at {pc}"
            );
        }
        if !ended {
            let successors = &self.cfg.blocks[block].successors;
            ensure!(successors.len() == 1, "block {block} falls off method");
            self.raw.ir.blocks[block].terminator = Terminator::Goto(successors[0]);
        }
        for target in self.raw.ir.blocks[block].terminator.successors() {
            self.incoming(target, block, None, &frame.locals, &frame.stack)?;
        }
        Ok(())
    }
}

pub(super) fn canonicalize(raw: &mut RawIr) {
    let mut parent: IdVec<ValueId, _> = raw.ir.values.ids().collect();
    loop {
        let mut changed = false;
        for value in raw.ir.values.ids() {
            if parent[value] != value {
                continue;
            }
            let ValueKind::Phi { inputs, .. } = &raw.ir.values[value].kind else {
                continue;
            };
            let mut candidate = None;
            let mut conflict = false;
            for input in inputs {
                let source = root(&parent, input.value);
                if source == value {
                    continue;
                }
                if candidate.is_some_and(|other| other != source) {
                    conflict = true;
                    break;
                }
                candidate = Some(source);
            }
            if !conflict && let Some(source) = candidate {
                parent[value] = source;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let roots: IdVec<ValueId, _> = raw
        .ir
        .values
        .ids()
        .map(|value| root(&parent, value))
        .collect();
    for access in &mut raw.local_accesses {
        access.value = roots[access.value];
        access.replaced = access.replaced.map(|value| roots[value]);
    }
    for value in &mut raw.ir.values {
        value.kind.map_operands(|value| roots[value]);
    }
    for block in &mut raw.ir.blocks {
        block.terminator.map_operands(|value| roots[value]);
    }
    for events in &mut raw.events {
        for event in events {
            if let EventKind::Action(action) = &mut event.kind {
                action.map_operands(|value| roots[value]);
            }
        }
    }
    // Collapsed phis are retained as unused arena entries. Their users already
    // refer to the canonical definition, so identity is stable for diagnostics.
    for value in raw.ir.values.ids() {
        if roots[value] != value {
            raw.ir.values[value].kind = ValueKind::Phi {
                inputs: vec![],
                slot: None,
            };
        }
    }
}

fn root(parent: &IdVec<ValueId, ValueId>, mut value: ValueId) -> ValueId {
    while parent[value] != value {
        value = parent[value];
    }
    value
}

fn binary_op(operation: BinaryOperation) -> BinaryOp {
    match operation {
        BinaryOperation::Add => BinaryOp::Add,
        BinaryOperation::Sub => BinaryOp::Sub,
        BinaryOperation::Mul => BinaryOp::Mul,
        BinaryOperation::Div => BinaryOp::Div,
        BinaryOperation::Rem => BinaryOp::Rem,
        BinaryOperation::Shl => BinaryOp::Shl,
        BinaryOperation::Shr => BinaryOp::Shr,
        BinaryOperation::Ushr => BinaryOp::Ushr,
        BinaryOperation::And => BinaryOp::And,
        BinaryOperation::Or => BinaryOp::Or,
        BinaryOperation::Xor => BinaryOp::Xor,
    }
}

fn comparison(kind: Comparison) -> CompareKind {
    match kind {
        Comparison::Long => CompareKind::Long,
        Comparison::FloatLess => CompareKind::FloatLess,
        Comparison::FloatGreater => CompareKind::FloatGreater,
        Comparison::DoubleLess => CompareKind::DoubleLess,
        Comparison::DoubleGreater => CompareKind::DoubleGreater,
    }
}

fn array_element(kind: u8, array: &JvmType) -> JvmType {
    match kind {
        0 => JvmType::Int,
        1 => JvmType::Long,
        2 => JvmType::Float,
        3 => JvmType::Double,
        4 => {
            if let JvmType::Array(element) = array {
                *element.clone()
            } else {
                JvmType::Object("java/lang/Object".into())
            }
        }
        5 => {
            if matches!(array, JvmType::Array(element) if **element == JvmType::Boolean) {
                JvmType::Boolean
            } else {
                JvmType::Byte
            }
        }
        6 => JvmType::Char,
        _ => JvmType::Short,
    }
}

fn class_type(name: &str) -> Result<JvmType> {
    if name.starts_with('[') {
        JvmType::parse(name)
    } else {
        Ok(JvmType::Object(name.to_owned()))
    }
}
