//! Ordered liveness of selected scheduled physical locals. Throw edges observe
//! state before a result commits; catch bindings define locals only on the
//! exceptional edge. Consumers share one transfer solver and effect vocabulary.
use super::MethodIr;
use super::local_effects::{self, Effect};
use crate::index::{BlockId, IdVec, LocalId};
use std::collections::BTreeSet;
use std::collections::VecDeque;

const MAX_GRAPH_WORDS: usize = 1 << 20;

/// All-entry facts for one immutable scheduled IR revision. Queries materialize
/// only the requested set, rather than allocating a tree set for every block.
pub(crate) struct LocalLiveness(Liveness);

impl LocalLiveness {
    pub fn analyze(ir: &MethodIr) -> crate::Result<Self> {
        let effects = effects(ir);
        let predecessors = predecessors(ir);
        let position: IdVec<LocalId, _> = (0..ir.locals.len()).map(Some).collect();
        let mut budget = 16_000_000;
        analyze(
            ir,
            &effects,
            &predecessors,
            &position,
            ir.locals.len(),
            &mut budget,
        )
        .map(Self)
        .ok_or_else(|| anyhow::anyhow!("scheduled local liveness exceeds its analysis budget"))
    }

    /// Values needed when control enters normally, including a normal edge to
    /// a block that also serves as an exception handler.
    pub fn at(&self, block: BlockId) -> BTreeSet<LocalId> {
        let mut locals = BTreeSet::new();
        self.0.entries[block].each(|local| {
            locals.insert(LocalId::from_index(local));
        });
        locals
    }

    /// The JVM supplies the caught exception only on this kind of entry.
    pub fn at_handler(&self, ir: &MethodIr, block: BlockId) -> BTreeSet<LocalId> {
        let mut locals = self.at(block);
        if let Some(local) = ir.blocks[block]
            .caught_exception
            .and_then(|value| ir.values[value].local)
        {
            locals.remove(&local);
        }
        locals
    }
}

pub(super) struct BlockEffects {
    pub(super) statements: Vec<Effect>,
    pub(super) terminal: Effect,
}

pub(super) fn effects(ir: &MethodIr) -> IdVec<BlockId, BlockEffects> {
    ir.blocks
        .iter()
        .map(|block| BlockEffects {
            statements: block
                .statements
                .iter()
                .map(|statement| local_effects::statement(ir, statement, true))
                .collect(),
            terminal: local_effects::terminator(ir, &block.terminator, true),
        })
        .collect()
}

pub(super) fn predecessors(ir: &MethodIr) -> IdVec<BlockId, Vec<BlockId>> {
    let mut predecessors: IdVec<BlockId, _> = vec![Vec::new(); ir.blocks.len()].into();
    for (block, body) in ir.blocks.iter_enumerated() {
        for target in body
            .terminator
            .successors()
            .into_iter()
            .chain(crate::cfg::dispatch::effective(&body.handlers).map(|edge| edge.target))
        {
            predecessors[target].push(block);
        }
    }
    predecessors
}

pub(super) struct Liveness {
    pub(super) entries: IdVec<BlockId, Bits>,
    all: Bits,
}

impl Liveness {
    pub(super) fn outgoing(
        &self,
        ir: &MethodIr,
        block: BlockId,
        position: &IdVec<LocalId, Option<usize>>,
    ) -> (Bits, Bits) {
        outgoing(ir, block, &self.entries, position, self.all.0.len())
    }

    pub(super) fn before_terminator(
        &self,
        ir: &MethodIr,
        effects: &IdVec<BlockId, BlockEffects>,
        block: BlockId,
        position: &IdVec<LocalId, Option<usize>>,
        budget: &mut usize,
    ) -> Option<Bits> {
        let (normal, exceptional) = self.outgoing(ir, block, position);
        let mut transfer = Transfer {
            uses: Bits::zero(self.all.0.len()),
            normal: self.all.clone(),
            exceptional: Bits::zero(self.all.0.len()),
        };
        summarize(
            &effects[block].terminal,
            &mut transfer,
            position,
            &self.all,
            budget,
        )?;
        let mut before = transfer.uses;
        before.union_intersection(&normal, &transfer.normal);
        before.union_intersection(&exceptional, &transfer.exceptional);
        Some(before)
    }
}

pub(super) fn analyze(
    ir: &MethodIr,
    effects: &IdVec<BlockId, BlockEffects>,
    predecessors: &IdVec<BlockId, Vec<BlockId>>,
    position: &IdVec<LocalId, Option<usize>>,
    count: usize,
    budget: &mut usize,
) -> Option<Liveness> {
    let words = count.div_ceil(64);
    if ir.blocks.len().saturating_mul(words) > MAX_GRAPH_WORDS {
        return None;
    }
    let mut all = Bits::zero(words);
    for local in 0..count {
        all.insert(local);
    }
    let mut transfers: IdVec<BlockId, _> = Vec::with_capacity(effects.len()).into();
    for block in effects {
        let mut transfer = Transfer {
            uses: Bits::zero(words),
            normal: all.clone(),
            exceptional: Bits::zero(words),
        };
        summarize(&block.terminal, &mut transfer, position, &all, budget)?;
        for statement in block.statements.iter().rev() {
            summarize(statement, &mut transfer, position, &all, budget)?;
        }
        transfers.push(transfer);
    }
    let mut live: IdVec<BlockId, _> = vec![Bits::zero(words); ir.blocks.len()].into();
    let mut pending: VecDeque<_> = (ir.blocks.ids()).rev().collect();
    let mut queued: IdVec<BlockId, _> = vec![true; ir.blocks.len()].into();
    while let Some(block) = pending.pop_front() {
        queued[block] = false;
        *budget = budget.checked_sub(words + 1)?;
        let (normal, exceptional) = outgoing(ir, block, &live, position, words);
        let transfer = &transfers[block];
        let mut before = transfer.uses.clone();
        before.union_intersection(&normal, &transfer.normal);
        before.union_intersection(&exceptional, &transfer.exceptional);
        if before != live[block] {
            live[block] = before;
            for &predecessor in &predecessors[block] {
                if !queued[predecessor] {
                    queued[predecessor] = true;
                    pending.push_back(predecessor);
                }
            }
        }
    }
    Some(Liveness { entries: live, all })
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Bits(pub(super) Vec<u64>);

impl Bits {
    pub(super) fn zero(words: usize) -> Self {
        Self(vec![0; words])
    }
    pub(super) fn one(words: usize, bit: usize) -> Self {
        let mut result = Self::zero(words);
        result.insert(bit);
        result
    }
    pub(super) fn insert(&mut self, bit: usize) {
        self.0[bit / 64] |= 1 << (bit % 64);
    }
    pub(super) fn remove(&mut self, bit: usize) {
        self.0[bit / 64] &= !(1 << (bit % 64));
    }
    pub(super) fn contains(&self, bit: usize) -> bool {
        self.0[bit / 64] & (1 << (bit % 64)) != 0
    }
    pub(super) fn union(&mut self, other: &Self) {
        for (left, right) in self.0.iter_mut().zip(&other.0) {
            *left |= *right;
        }
    }
    pub(super) fn union_intersection(&mut self, left: &Self, right: &Self) {
        for ((output, left), right) in self.0.iter_mut().zip(&left.0).zip(&right.0) {
            *output |= left & right;
        }
    }
    pub(super) fn intersects(&self, other: &Self) -> bool {
        self.0
            .iter()
            .zip(&other.0)
            .any(|(left, right)| left & right != 0)
    }
    pub(super) fn each(&self, mut visit: impl FnMut(usize)) {
        for (word_index, &word) in self.0.iter().enumerate() {
            let mut word = word;
            while word != 0 {
                let bit = word.trailing_zeros() as usize;
                visit(word_index * 64 + bit);
                word &= word - 1;
            }
        }
    }
}

// A block's exact may-liveness transfer is distributive over unions:
// uses | (normal_exit & normal) | (exception_exit & exceptional). Summarizing
// ordered events once avoids walking every expression on every CFG iteration.
// A write kills all three components; each Throw adds the exceptional input at
// that precise point before earlier writes are processed.
#[derive(Clone)]
pub(super) struct Transfer {
    pub(super) uses: Bits,
    pub(super) normal: Bits,
    pub(super) exceptional: Bits,
}

pub(super) fn summarize(
    effect: &Effect,
    facts: &mut Transfer,
    position: &IdVec<LocalId, Option<usize>>,
    all: &Bits,
    budget: &mut usize,
) -> Option<()> {
    if *budget == 0 {
        return None;
    }
    *budget -= 1;
    match effect {
        Effect::Read(local) => {
            if let Some(local) = position[*local] {
                facts.uses.insert(local);
            }
        }
        Effect::Write { local, .. } => {
            if let Some(local) = position[*local] {
                facts.uses.remove(local);
                facts.normal.remove(local);
                facts.exceptional.remove(local);
            }
        }
        Effect::Throw => facts.exceptional.union(all),
        Effect::Sequence(effects) => {
            for effect in effects.iter().rev() {
                summarize(effect, facts, position, all, budget)?;
            }
        }
        Effect::Choice(left, right) => {
            let mut other = facts.clone();
            summarize(left, facts, position, all, budget)?;
            summarize(right, &mut other, position, all, budget)?;
            facts.uses.union(&other.uses);
            facts.normal.union(&other.normal);
            facts.exceptional.union(&other.exceptional);
        }
        Effect::Empty => {}
    }
    Some(())
}

fn outgoing(
    ir: &MethodIr,
    block: BlockId,
    live: &IdVec<BlockId, Bits>,
    position: &IdVec<LocalId, Option<usize>>,
    words: usize,
) -> (Bits, Bits) {
    let mut normal = Bits::zero(words);
    for target in ir.blocks[block].terminator.successors() {
        normal.union(&live[target]);
    }
    let mut exceptional = Bits::zero(words);
    for handler in crate::cfg::dispatch::effective(&ir.blocks[block].handlers) {
        let mut edge = live[handler.target].clone();
        // This definition belongs to the exception edge. A normal predecessor
        // entering the very same handler block must retain the incoming local.
        if let Some(local) = ir.blocks[handler.target]
            .caught_exception
            .and_then(|value| ir.values[value].local)
            .and_then(|local| position[local])
        {
            edge.remove(local);
        }
        exceptional.union(&edge);
    }
    (normal, exceptional)
}
