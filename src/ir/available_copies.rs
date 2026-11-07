//! Forward must-analysis of copies and literal contents of scheduled locals.
//!
//! SSA identities can be re-evaluated on every loop iteration. Facts therefore
//! describe current local contents. A write removes its destination from its
//! previous content class; the other snapshots still agree. Exceptional entries
//! meet snapshots at throwing operations, before their result stores commit.
//! Tracking policies select facts, never different join/write rules.
use super::*;
use crate::index::IdVec;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

// Every nonliteral class points directly to its smallest local. A known
// non-null representative keeps its own entry, including for a singleton.
type Facts = HashMap<LocalId, Content>;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum LiteralKey {
    Null,
    Int(i32),
    Long(i64),
    Float(u32),
    Double(u64),
    String(Vec<u16>),
    Class(JvmType),
}

impl From<&Literal> for LiteralKey {
    fn from(value: &Literal) -> Self {
        match value {
            Literal::Null => Self::Null,
            Literal::Int(value) => Self::Int(*value),
            Literal::Long(value) => Self::Long(*value),
            Literal::Float(value) => Self::Float(value.to_bits()),
            Literal::Double(value) => Self::Double(value.to_bits()),
            Literal::String(value) => Self::String(value.clone()),
            Literal::Class(value) => Self::Class(value.clone()),
        }
    }
}

#[derive(Clone, Copy)]
enum ActionKind {
    Kill { non_null: bool },
    Copy(LocalId),
    Literal(usize),
}

#[derive(Clone, Copy)]
struct Action {
    local: LocalId,
    statement: usize,
    kind: ActionKind,
}

#[derive(Clone, Copy)]
struct Predecessor {
    block: BlockId,
    exceptional: bool,
}

#[derive(Clone, Default)]
struct Completion {
    normal: Option<Rc<Facts>>,
    exceptional: Option<Rc<Facts>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tracking {
    All,
    RepeatedCopies,
    RepeatedStores,
}

/// Facts refer to the exact scheduled statement positions used to build them.
/// Build a new analysis after mutating the graph or its statement lists.
pub(crate) struct AvailableCopies {
    entries: IdVec<BlockId, Option<Rc<Facts>>>,
    actions: IdVec<BlockId, Vec<Action>>,
    literals: HashMap<LiteralKey, usize>,
}

/// One exact pre-statement content view. It can answer several peer queries
/// without replaying the statement prefix.
pub(super) struct AssignmentContents {
    facts: Rc<Facts>,
    rhs: Content,
}

impl AssignmentContents {
    pub(super) fn held_by(&self, local: LocalId) -> bool {
        content(&self.facts, local) == self.rhs
    }
}

impl AvailableCopies {
    pub(crate) fn analyze(ir: &MethodIr) -> Self {
        analyze(ir, Tracking::All)
    }

    /// `before == statements.len()` queries the state before the terminator.
    /// This proves read identity only; it never licenses renaming destinations.
    pub(crate) fn reads_same(
        &self,
        block: BlockId,
        before: usize,
        left: LocalId,
        right: LocalId,
    ) -> bool {
        if left == right {
            return true;
        }
        let Some(facts) = self.before(block, before) else {
            return false;
        };
        content(&facts, left) == content(&facts, right)
    }

    /// Only normal-completion facts at this exact source position participate.
    /// In particular, a literal initializer is not a timeless property of its
    /// ValueId: later writes and loop reentry can replace the local's contents.
    pub(crate) fn reads_literal(
        &self,
        block: BlockId,
        before: usize,
        local: LocalId,
        literal: &Literal,
    ) -> bool {
        let Some(&expected) = self.literals.get(&LiteralKey::from(literal)) else {
            return false;
        };
        self.before(block, before)
            .is_some_and(|facts| content(&facts, local) == Content::Literal(expected))
    }

    pub(crate) fn reads_non_null(&self, block: BlockId, before: usize, local: LocalId) -> bool {
        self.before(block, before)
            .is_some_and(|facts| matches!(content(&facts, local), Content::NonNull(_)))
    }

    pub(super) fn assignment_contents(
        &self,
        ir: &MethodIr,
        block: BlockId,
        statement: usize,
    ) -> Option<AssignmentContents> {
        // This is the RHS's normal-completion content, not a claim that the
        // destination already contains it or that a throwing conversion commits.
        let rhs = assignment_rhs(ir, ir.blocks.get(block)?.statements.get(statement)?)?;
        let facts = self.before(block, statement)?;
        let rhs = match rhs {
            AssignmentRhs::Copy(local) => content(&facts, local),
            AssignmentRhs::Literal(literal) => Content::Literal(*self.literals.get(&literal)?),
        };
        Some(AssignmentContents { facts, rhs })
    }

    fn before(&self, block: BlockId, before: usize) -> Option<Rc<Facts>> {
        let mut facts = self.entries.get(block).cloned().flatten()?;
        for &action in &self.actions[block] {
            if action.statement >= before {
                break;
            }
            transfer(&mut facts, action);
        }
        Some(facts)
    }
}

enum AssignmentRhs {
    Copy(LocalId),
    Literal(LiteralKey),
}

pub(super) fn has_assignment_contents(ir: &MethodIr, statement: &Statement) -> bool {
    assignment_rhs(ir, statement).is_some()
}

pub(super) fn is_literal_store(ir: &MethodIr, statement: &Statement) -> bool {
    matches!(
        assignment_rhs(ir, statement),
        Some(AssignmentRhs::Literal(_))
    )
}

fn assignment_rhs(ir: &MethodIr, statement: &Statement) -> Option<AssignmentRhs> {
    if let Some((_, source)) = copy(ir, statement) {
        return Some(AssignmentRhs::Copy(source));
    }
    let Statement::Assign { value, .. } = statement else {
        return None;
    };
    if matches!(
        ir.values[value.value()].kind,
        ValueKind::Literal(Literal::String(_) | Literal::Class(_))
    ) {
        return None;
    }
    let (_, literal) = literal_assignment(ir, statement)?;
    Some(AssignmentRhs::Literal(literal))
}

fn analyze(ir: &MethodIr, tracking: Tracking) -> AvailableCopies {
    // Removal tracks repeated assignments, avoiding accumulated facts for dead
    // temporaries. Read-equivalence queries keep every identity assignment.
    let mut counts = HashMap::new();
    let mut literals = HashMap::new();
    let mut literal_assignments = HashMap::new();
    let mut allocated = HashSet::new();
    for (block_id, block) in ir.blocks.iter_enumerated() {
        for (index, statement) in block.statements.iter().enumerate() {
            if tracking == Tracking::All {
                allocated.extend(non_null_assignment(ir, statement));
            }
            if let Some(pair) = copy(ir, statement) {
                *counts.entry(pair).or_insert(0usize) += 1;
            } else if tracking != Tracking::RepeatedCopies
                && let Some((destination, literal)) = literal_assignment(ir, statement)
                && (tracking == Tracking::All
                    || !matches!(literal, LiteralKey::String(_) | LiteralKey::Class(_)))
            {
                let next = literals.len();
                let literal = *literals.entry(literal).or_insert(next);
                literal_assignments.insert((block_id, index), (destination, literal));
            }
        }
    }
    let candidates: HashSet<_> = counts
        .into_iter()
        .filter_map(|(pair, count)| (tracking == Tracking::All || count > 1).then_some(pair))
        .collect();
    if tracking == Tracking::RepeatedStores {
        let mut counts = HashMap::new();
        for &pair in literal_assignments.values() {
            *counts.entry(pair).or_insert(0usize) += 1;
        }
        literal_assignments.retain(|_, pair| counts[pair] > 1);
    }
    if candidates.is_empty() && literal_assignments.is_empty() && allocated.is_empty() {
        return AvailableCopies {
            entries: IdVec::new(),
            actions: IdVec::new(),
            literals,
        };
    }
    let mut watched: HashSet<_> = candidates
        .iter()
        .flat_map(|&(destination, source)| [destination, source])
        .collect();
    watched.extend(
        literal_assignments
            .values()
            .map(|&(destination, _)| destination),
    );
    watched.extend(allocated);
    // Captures are uncommon. Ordinary repeated-copy elimination need not build
    // expression trees just to discover that no expression can write a local.
    let inline_writes = ir
        .values
        .iter()
        .any(|value| value.kind.local_write().is_some());
    let expression_writes = |value, watched: &HashSet<_>, writes: &mut Vec<_>| {
        if inline_writes {
            super::local_effects::expression(ir, value).writes(&mut |local| {
                if watched.contains(&local) {
                    writes.push(local);
                }
            });
        }
    };
    let actions: IdVec<BlockId, Vec<_>> = ir
        .blocks
        .iter_enumerated()
        .map(|(block_id, block)| {
            let mut actions = vec![];
            for (index, statement) in block.statements.iter().enumerate() {
                if let Some((destination, source)) =
                    copy(ir, statement).filter(|pair| candidates.contains(pair))
                {
                    actions.push(Action {
                        local: destination,
                        kind: ActionKind::Copy(source),
                        statement: index,
                    });
                    continue;
                }
                if let Some(&(destination, literal)) = literal_assignments.get(&(block_id, index)) {
                    actions.push(Action {
                        local: destination,
                        kind: ActionKind::Literal(literal),
                        statement: index,
                    });
                    continue;
                }
                let mut writes = vec![];
                match statement {
                    Statement::Assign { local, value } => {
                        expression_writes(*value, &watched, &mut writes);
                        if watched.contains(local) {
                            writes.push(*local);
                        }
                    }
                    Statement::Eval(value) => expression_writes(
                        super::ValueUse::Definition(*value),
                        &watched,
                        &mut writes,
                    ),
                    _ => {
                        for value in statement.operands() {
                            expression_writes(
                                super::ValueUse::Reference(value),
                                &watched,
                                &mut writes,
                            );
                        }
                    }
                }
                writes.sort_unstable();
                writes.dedup();
                actions.extend(writes.into_iter().map(|local| Action {
                    local,
                    statement: index,
                    kind: ActionKind::Kill {
                        non_null: tracking == Tracking::All
                            && non_null_assignment(ir, statement) == Some(local),
                    },
                }));
            }
            let mut writes = vec![];
            for value in block.terminator.operands() {
                expression_writes(super::ValueUse::Reference(value), &watched, &mut writes);
            }
            writes.sort_unstable();
            writes.dedup();
            actions.extend(writes.into_iter().map(|local| Action {
                local,
                statement: block.statements.len(),
                kind: ActionKind::Kill { non_null: false },
            }));
            actions
        })
        .collect();
    let count = ir.blocks.len();
    let mut predecessors: IdVec<BlockId, _> = vec![vec![]; count].into();
    let mut seeds: IdVec<BlockId, _> = vec![false; count].into();
    seeds[ir.entry] = true;
    for (source, block) in ir.blocks.iter_enumerated() {
        for target in block.terminator.successors() {
            predecessors[target].push(Predecessor {
                block: source,
                exceptional: false,
            });
        }
        for handler in crate::cfg::dispatch::effective(&block.handlers) {
            predecessors[handler.target].push(Predecessor {
                block: source,
                exceptional: true,
            });
        }
    }
    // Reuse the scheduled expression's ordered effects. A handler sees every
    // possible throw, including failures before an embedded local write and
    // failures after it. Only protected blocks need these transient trees.
    let exceptional: IdVec<BlockId, Vec<_>> = ir
        .blocks
        .iter()
        .map(|block| {
            if block.handlers.is_empty() {
                Vec::new()
            } else {
                block
                    .statements
                    .iter()
                    .map(|statement| super::local_effects::statement(ir, statement, true))
                    .chain(std::iter::once(super::local_effects::terminator(
                        ir,
                        &block.terminator,
                        true,
                    )))
                    .collect()
            }
        })
        .collect();
    let caught: IdVec<BlockId, _> = ir
        .blocks
        .iter()
        .map(|block| {
            block
                .caught_exception
                .and_then(|value| ir.values[value].local)
        })
        .collect();
    let empty = Rc::new(Facts::default());
    // None is the lattice's initial top. Empty is a known lack of available
    // copies. Sharing unchanged sparse maps makes straight-line regions cheap.
    let mut outputs: IdVec<BlockId, _> = vec![Completion::default(); count].into();
    let mut pending: VecDeque<_> = ir.blocks.ids().collect();
    let mut queued: IdVec<BlockId, _> = vec![true; count].into();
    while let Some(block) = pending.pop_front() {
        queued[block] = false;
        let Some(mut facts) = incoming(block, &predecessors, &seeds, &outputs, &empty, &caught)
        else {
            continue;
        };
        let mut escaping = None;
        if exceptional[block].is_empty() {
            for &action in &actions[block] {
                transfer(&mut facts, action);
            }
        } else {
            let mut actions = actions[block].iter().peekable();
            for (index, effect) in exceptional[block].iter().enumerate() {
                let mut partial = facts.clone();
                exceptional_transfer(effect, &mut partial, &mut escaping);
                while actions
                    .peek()
                    .is_some_and(|action| action.statement == index)
                {
                    transfer(&mut facts, *actions.next().unwrap());
                }
            }
        }
        if outputs[block]
            .normal
            .as_ref()
            .is_some_and(|old| **old == *facts)
            && outputs[block].exceptional == escaping
        {
            continue;
        }
        outputs[block] = Completion {
            normal: Some(facts),
            exceptional: escaping,
        };
        let targets = ir.blocks[block].terminator.successors().into_iter().chain(
            crate::cfg::dispatch::effective(&ir.blocks[block].handlers)
                .map(|handler| handler.target),
        );
        for target in targets {
            if !queued[target] {
                queued[target] = true;
                pending.push_back(target);
            }
        }
    }
    let entries = ir
        .blocks
        .ids()
        .map(|block| incoming(block, &predecessors, &seeds, &outputs, &empty, &caught))
        .collect();
    AvailableCopies {
        entries,
        actions,
        literals,
    }
}

pub(super) fn eliminate(raw: &mut super::lift::RawIr) {
    eliminate_tracked(raw, Tracking::RepeatedCopies);
}

pub(super) fn eliminate_after_coalescing(raw: &mut super::lift::RawIr) {
    eliminate_tracked(raw, Tracking::RepeatedStores);
}

fn eliminate_tracked(raw: &mut super::lift::RawIr, tracking: Tracking) {
    let analysis = analyze(&raw.ir, tracking);
    for (block, actions) in analysis.actions.iter_enumerated() {
        let Some(mut facts) = analysis.entries[block].clone() else {
            continue;
        };
        let mut removed = vec![];
        for &action in actions {
            let repeated = match action.kind {
                ActionKind::Copy(source) => Some(content(&facts, source)),
                ActionKind::Literal(literal) => Some(Content::Literal(literal)),
                ActionKind::Kill { .. } => None,
            };
            if repeated == Some(content(&facts, action.local)) {
                removed.push(action.statement);
            }
            transfer(&mut facts, action);
        }
        if removed.is_empty() {
            continue;
        }
        raw.retain_statements(block, |index| removed.binary_search(&index).is_err());
    }
}

fn copy(ir: &MethodIr, statement: &Statement) -> Option<(LocalId, LocalId)> {
    let Statement::Assign { local, value } = statement else {
        return None;
    };
    let source = value.local(ir)?;
    let destination_type = &ir.locals[*local].ty;
    let source_type = &ir.locals[source].ty;
    // Reference conversions retain identity on normal completion. Primitive
    // narrowing/boolean conversions are operations, not identity relations.
    (source != *local
        && (destination_type == source_type
            || (destination_type.is_reference() && source_type.is_reference())))
    .then_some((*local, source))
}

fn literal_assignment(ir: &MethodIr, statement: &Statement) -> Option<(LocalId, LiteralKey)> {
    let Statement::Assign { local, value } = *statement else {
        return None;
    };
    if value.local(ir).is_some() {
        return None;
    }
    let value = value.value();
    let ValueKind::Literal(literal) = &ir.values[value].kind else {
        return None;
    };
    let from = &ir.values[value].ty;
    let to = &ir.locals[local].ty;
    let integral = |value: i32, ty: &JvmType| match ty {
        JvmType::Boolean => matches!(value, 0 | 1),
        JvmType::Byte => i8::try_from(value).is_ok(),
        JvmType::Short => i16::try_from(value).is_ok(),
        JvmType::Char => u16::try_from(value).is_ok(),
        JvmType::Int => true,
        _ => false,
    };
    // Track only value-preserving primitive conversions. In particular, a
    // truncated byte, int-to-float rounding, or int-to-boolean truth conversion
    // cannot inherit the literal's original numeric identity.
    let exact = match literal {
        Literal::Null => to.is_reference(),
        Literal::Int(value) => integral(*value, from) && integral(*value, to),
        Literal::Long(_) => *from == JvmType::Long && *to == JvmType::Long,
        Literal::Float(_) => *from == JvmType::Float && *to == JvmType::Float,
        Literal::Double(_) => *from == JvmType::Double && *to == JvmType::Double,
        Literal::String(_) | Literal::Class(_) => from.is_reference() && to.is_reference(),
    };
    exact.then(|| (local, LiteralKey::from(literal)))
}

fn non_null_assignment(ir: &MethodIr, statement: &Statement) -> Option<LocalId> {
    let Statement::Assign { local, value } = *statement else {
        return None;
    };
    (value.local(ir).is_none()
        && matches!(
            ir.values[value.value()].kind,
            ValueKind::New { .. } | ValueKind::NewArray { .. } | ValueKind::ArrayInitializer { .. }
        ))
    .then_some(local)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Content {
    Local(LocalId),
    NonNull(LocalId),
    Literal(usize),
}

impl Content {
    fn representative(self) -> Option<LocalId> {
        match self {
            Self::Local(local) | Self::NonNull(local) => Some(local),
            Self::Literal(_) => None,
        }
    }

    fn represented_by(self, local: LocalId) -> Self {
        if matches!(self, Self::NonNull(_)) {
            Self::NonNull(local)
        } else {
            Self::Local(local)
        }
    }
}

fn content(facts: &Facts, local: LocalId) -> Content {
    facts.get(&local).copied().unwrap_or(Content::Local(local))
}

fn common_contents(left: &Facts, right: &Facts) -> Facts {
    // Refine classes by pairs of incoming identities. A shared-object copy on
    // one predecessor and two nulls on another still prove equality at the join.
    // A singleton on the left cannot contribute a relation, so only its
    // represented locals (including class representatives) need signatures.
    let locals: HashSet<_> = left
        .keys()
        .copied()
        .chain(left.values().filter_map(|content| content.representative()))
        .collect();
    let mut groups = HashMap::<(Content, Content), Vec<LocalId>>::new();
    for local in locals {
        groups
            .entry((content(left, local), content(right, local)))
            .or_default()
            .push(local);
    }
    let mut result = Facts::new();
    for ((left, right), locals) in groups {
        let shared = match (left, right) {
            (Content::Literal(left), Content::Literal(right)) if left == right => {
                Content::Literal(left)
            }
            (Content::NonNull(_), Content::NonNull(_)) => {
                Content::NonNull(*locals.iter().min().unwrap())
            }
            _ => Content::Local(*locals.iter().min().unwrap()),
        };
        for local in locals {
            if shared != Content::Local(local) {
                result.insert(local, shared);
            }
        }
    }
    result
}

fn incoming(
    block: BlockId,
    predecessors: &IdVec<BlockId, Vec<Predecessor>>,
    seeds: &IdVec<BlockId, bool>,
    outputs: &IdVec<BlockId, Completion>,
    empty: &Rc<Facts>,
    caught: &IdVec<BlockId, Option<LocalId>>,
) -> Option<Rc<Facts>> {
    if seeds[block] || predecessors[block].is_empty() {
        return Some(empty.clone());
    }
    let known = predecessors[block].iter().filter_map(|predecessor| {
        let completion = &outputs[predecessor.block];
        if predecessor.exceptional {
            &completion.exceptional
        } else {
            &completion.normal
        }
        .as_ref()
    });
    let mut result = None;
    for facts in known {
        meet(&mut result, facts);
    }
    let mut result = result?;
    if let Some(local) = caught[block] {
        kill(&mut result, local);
    }
    Some(result)
}

fn meet(result: &mut Option<Rc<Facts>>, facts: &Rc<Facts>) {
    *result = Some(match result {
        None => facts.clone(),
        Some(old) => intersection(old, facts),
    });
}

fn intersection(left: &Rc<Facts>, right: &Rc<Facts>) -> Rc<Facts> {
    if left.is_empty() || Rc::ptr_eq(left, right) || left == right {
        left.clone()
    } else if right.is_empty() {
        right.clone()
    } else {
        Rc::new(common_contents(left, right))
    }
}

fn kill(facts: &mut Rc<Facts>, local: LocalId) {
    transfer(
        facts,
        Action {
            local,
            statement: 0,
            kind: ActionKind::Kill { non_null: false },
        },
    );
}

/// Internal writes can precede a later failure of the same expression. Forget
/// their old identities at the precise write, while keeping unaffected local
/// snapshots. Completed statements still use the canonical normal transfers.
fn exceptional_transfer(
    effect: &super::local_effects::Effect,
    facts: &mut Rc<Facts>,
    escaping: &mut Option<Rc<Facts>>,
) {
    use super::local_effects::Effect;
    match effect {
        Effect::Throw => meet(escaping, facts),
        Effect::Write { local, .. } => kill(facts, *local),
        Effect::Sequence(effects) => {
            for effect in effects {
                exceptional_transfer(effect, facts, escaping);
            }
        }
        Effect::Choice(left, right) => {
            let mut other = facts.clone();
            exceptional_transfer(left, facts, escaping);
            exceptional_transfer(right, &mut other, escaping);
            *facts = intersection(facts, &other);
        }
        Effect::Empty | Effect::Read(_) => (),
    }
}

fn transfer(facts: &mut Rc<Facts>, action: Action) {
    let written = action.local;
    let assigned = match action.kind {
        ActionKind::Copy(source) => Some(content(facts, source)),
        ActionKind::Literal(literal) => Some(Content::Literal(literal)),
        ActionKind::Kill { non_null } => non_null.then_some(Content::NonNull(written)),
    };
    // Allocating into an already non-null representative still creates a new
    // object. Its previous snapshots must separate before retaining the fact.
    if !matches!(action.kind, ActionKind::Kill { non_null: true })
        && assigned == Some(content(facts, written))
        || assigned.is_none()
            && !facts.contains_key(&written)
            && !facts
                .values()
                .any(|value| *value == Content::Local(written))
    {
        return;
    }
    let facts = Rc::make_mut(facts);
    // Removing the representative must preserve the other captured values.
    let old = content(facts, written);
    facts.remove(&written);
    if old.representative() == Some(written)
        && let Some(replacement) = facts
            .iter()
            .filter_map(|(&local, &value)| (value == old).then_some(local))
            .min()
    {
        for value in facts.values_mut() {
            if *value == old {
                *value = old.represented_by(replacement);
            }
        }
        if facts[&replacement] == Content::Local(replacement) {
            facts.remove(&replacement);
        }
    }
    // Equal-content writes returned above, so removing written cannot change
    // the source's class representative.
    let Some(assigned) = assigned else { return };
    if let Some(representative) = assigned.representative()
        && written < representative
    {
        for value in facts.values_mut() {
            if *value == assigned {
                *value = assigned.represented_by(written);
            }
        }
        facts.insert(representative, assigned.represented_by(written));
        if matches!(assigned, Content::NonNull(_)) {
            facts.insert(written, Content::NonNull(written));
        }
    } else {
        facts.insert(written, assigned);
    }
}

#[cfg(test)]
#[path = "../../tests/unit/available_copies.rs"]
mod tests;
