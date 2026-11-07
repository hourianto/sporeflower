//! Recover postfix results before storage coalescing. Keep the old result and
//! the new value as separate snapshots, relocate only the checked local write,
//! and let ordinary interference decide whether their storage can be shared.
use super::lift::RawIr;
use super::local_effects::{self, Effect};
use super::scheduled::{self, ValueUse};
use super::{BinaryOp, JvmType, Literal, LocalId, MethodIr, Statement, ValueId, ValueKind};
use crate::index::{BlockId, IdVec};
use std::collections::BTreeSet;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Consumer {
    Value(ValueId),
    Statement(usize),
    Terminator,
}

pub(super) fn recover(raw: &mut RawIr) {
    // IINC and explicit arithmetic committed back to their input binding
    // describe the same old/new snapshots. An independent `next = current + 1`
    // is not evidence of mutation. Delayed commits, including alternating
    // cursor slots, retain ownership in a closed chain of original stores and
    // normal frame phis. The eventual source storage identity still needs the
    // relocation proof.
    let mut stored: IdVec<ValueId, bool> = vec![false; raw.ir.values.len()].into();
    for access in &raw.local_accesses {
        if access.store {
            stored[access.value] = true;
        }
    }
    let mut increments: Vec<_> = raw
        .local_accesses
        .iter()
        .filter_map(|access| {
            let value = &raw.ir.values[access.value];
            let (left, increment) = unit_update(&raw.ir, access.value)?;
            (access.store
                && (access.replaced == Some(left)
                    || carried_update(&raw.ir, &stored, left, access.value)))
            .then_some((value.block, value.pc, access.value, left, increment))
        })
        .collect();
    // Recover the last update first. Its earlier pure arithmetic may have
    // been scheduled before preceding reads. Once its own consumer owns
    // that definition, an enclosing update can share the resulting lifetime.
    increments
        .sort_unstable_by_key(|&(block, pc, next, _, _)| (block, std::cmp::Reverse(pc), next));
    increments.dedup();
    if increments.is_empty() {
        return;
    }
    let Ok(liveness) = super::scheduled_liveness::LocalLiveness::analyze(&raw.ir) else {
        return;
    };
    // Editing an operation's operands must not change another block's copy of
    // a pure expression. Same-block repetitions are counted by the use walk.
    let mut escaped = BTreeSet::new();
    for (block, body) in raw.ir.blocks.iter_enumerated() {
        for value in body
            .statements
            .iter()
            .flat_map(|statement| scheduled::statement_values(&raw.ir, statement).operations())
            .chain(body.terminator.operands().into_iter().flat_map(|value| {
                scheduled::values(&raw.ir, ValueUse::Reference(value)).operations()
            }))
        {
            if raw.ir.values[value].block != block {
                escaped.insert(value);
            }
        }
    }
    let exits: IdVec<BlockId, [BTreeSet<LocalId>; 2]> = raw
        .ir
        .blocks
        .iter()
        .map(|block| {
            [
                block
                    .terminator
                    .successors()
                    .into_iter()
                    .flat_map(|target| liveness.at(target))
                    .collect(),
                crate::cfg::dispatch::effective(&block.handlers)
                    .flat_map(|edge| liveness.at_handler(&raw.ir, edge.target))
                    .collect(),
            ]
        })
        .collect();
    let mut budget = 1_000_000;
    for (block, pc, next, before, increment) in increments {
        if budget == 0 {
            break;
        }
        let Some(local) = raw.ir.values[before].local else {
            continue;
        };
        let previous = raw.ir.values[next].local;
        if raw.ir.locals[local].ty != JvmType::Int
            || raw.ir.values[next].ty != JvmType::Int
            || previous.is_some_and(|saved| {
                saved == local
                    || raw.ir.locals[saved].ty != JvmType::Int
                    || raw.ir.locals[saved].parameter.is_some()
                    || raw.ir.locals[saved].is_this
            })
        {
            continue;
        }
        let targets = consumers(raw, block, before, next, pc, &escaped, &mut budget);
        let [target] = targets.as_slice() else {
            continue;
        };
        let target = *target;
        let definitions: Vec<_> = raw.ir.blocks[block].statements.iter().enumerate().filter_map(|(index, statement)| {
            matches!(statement, Statement::Assign { value: ValueUse::Definition(value), .. } if *value == next).then_some(index)
        }).collect();
        if previous.is_some() && definitions.len() != 1 {
            continue;
        }
        let original: Vec<_> = raw.ir.blocks[block]
            .statements
            .iter()
            .cloned()
            .zip(raw.statement_pcs(block).iter().copied())
            .collect();
        let local_count = raw.ir.locals.len();
        let saved = previous.unwrap_or_else(|| {
            raw.ir.locals.push(super::Local {
                name: format!("v{local_count}"),
                name_hint: None,
                ty: JvmType::Int,
                parameter: None,
                is_this: false,
                needs_default: false,
            })
        });
        raw.ir.values[next].local = Some(saved);
        let update = raw.ir.values.push(super::Value {
            local: None,
            kind: ValueKind::LocalUpdate {
                local: saved,
                value: next,
                increment,
                postfix: true,
            },
            ty: JvmType::Int,
            pc,
            block,
        });
        target.replace(raw, block, before, update);
        let copy = Statement::Assign {
            local: saved,
            value: ValueUse::Reference(before),
        };
        if let Some(&definition) = definitions.first() {
            raw.ir.blocks[block].statements[definition] = copy;
        } else {
            let at = raw
                .statement_pcs(block)
                .iter()
                .position(|&origin| origin >= pc)
                .unwrap_or(original.len());
            let mut statements: Vec<_> = raw.ir.blocks[block]
                .statements
                .iter()
                .cloned()
                .zip(raw.statement_pcs(block).iter().copied())
                .collect();
            statements.insert(at, (copy, pc));
            raw.set_statements(block, statements);
        }
        let effects: Vec<_> = raw.ir.blocks[block]
            .statements
            .iter()
            .map(|statement| local_effects::statement(&raw.ir, statement, true))
            .chain(std::iter::once(local_effects::terminator(
                &raw.ir,
                &raw.ir.blocks[block].terminator,
                true,
            )))
            .collect();
        let mut proof = Relocation {
            old: local,
            saved,
            before,
            next,
            observed: exits[block][1].contains(&saved),
            budget: &mut budget,
        };
        let mut state = Phase::Before;
        let valid = effects
            .iter()
            .all(|effect| proof.effect(effect, &mut state))
            && state == Phase::Done;
        if !valid {
            // Only this block and the consuming operand were speculative.
            raw.set_statements(block, original);
            target.replace(raw, block, update, before);
            raw.ir.values[next].local = previous;
            raw.ir.values.truncate(update.index());
            raw.ir.locals.truncate(local_count);
            continue;
        }
        // A confined successor snapshot can share storage immediately. This
        // releases its predecessor for the remaining backward traversal; an
        // escaping snapshot waits for the ordinary whole-method coalescer.
        if super::coalesce::confined_copy(
            &raw.ir,
            local,
            saved,
            &effects,
            &exits[block],
            &mut budget,
        ) {
            let copies: BTreeSet<_> = raw.ir.blocks[block]
                .statements
                .iter()
                .enumerate()
                .filter_map(|(index, statement)| {
                    let Statement::Assign {
                        local: destination,
                        value,
                    } = statement
                    else {
                        return None;
                    };
                    let source = value.local(&raw.ir)?;
                    (source != *destination
                        && [local, saved].contains(&source)
                        && [local, saved].contains(destination))
                    .then_some(index)
                })
                .collect();
            raw.retain_statements(block, |index| !copies.contains(&index));
            for statement in &mut raw.ir.blocks[block].statements {
                if let Statement::Assign { local: target, .. } = statement
                    && *target == saved
                {
                    *target = local;
                }
            }
            for value in &mut raw.ir.values {
                if value.local == Some(saved) {
                    value.local = Some(local);
                }
                if let Some((target, _)) = value.kind.local_write_mut()
                    && *target == saved
                {
                    *target = local;
                }
            }
        }
    }
}

fn unit_update(ir: &MethodIr, value: ValueId) -> Option<(ValueId, bool)> {
    let ValueKind::Binary {
        op: op @ (BinaryOp::Add | BinaryOp::Sub),
        left,
        right,
    } = ir.values[value].kind
    else {
        return None;
    };
    let ValueKind::Literal(Literal::Int(amount @ (-1 | 1))) = ir.values[right].kind else {
        return None;
    };
    Some((left, (amount == 1) == (op == BinaryOp::Add)))
}

/// The update already depends on `before`; reaching it again through normal
/// frame inputs and stored unit updates closes an actual carried recurrence.
/// Independent neighboring arithmetic never feeds that storage cycle. Frame
/// phis also preserve this evidence when the old value is dead at a delayed
/// commit, and allow an arbitrary number of successive cursor increments.
fn carried_update(
    ir: &MethodIr,
    stored: &IdVec<ValueId, bool>,
    before: ValueId,
    next: ValueId,
) -> bool {
    let mut seen = BTreeSet::new();
    let mut pending = vec![before];
    while let Some(value) = pending.pop() {
        if value == next {
            return true;
        }
        if !seen.insert(value) {
            continue;
        }
        if let ValueKind::Phi {
            slot: Some(_),
            inputs,
        } = &ir.values[value].kind
        {
            pending.extend(
                inputs
                    .iter()
                    .filter(|input| input.throw_pc.is_none())
                    .map(|input| input.value),
            );
        } else if stored[value]
            && let Some((previous, _)) = unit_update(ir, value)
        {
            pending.push(previous);
        }
    }
    false
}

impl Consumer {
    fn replace(self, raw: &mut RawIr, block: BlockId, old: ValueId, replacement: ValueId) {
        let map = |value| if value == old { replacement } else { value };
        match self {
            Self::Value(value) => raw.ir.values[value].kind.map_operands(map),
            Self::Statement(index) => raw.ir.blocks[block].statements[index].map_operands(map),
            Self::Terminator => raw.ir.blocks[block].terminator.map_operands(map),
        }
    }
}

fn consumers(
    raw: &RawIr,
    block: BlockId,
    before: ValueId,
    next: ValueId,
    pc: u32,
    escaped: &BTreeSet<ValueId>,
    budget: &mut usize,
) -> Vec<Consumer> {
    let mut pending = Vec::new();
    let mut targets = Vec::new();
    for (index, statement) in raw.ir.blocks[block].statements.iter().enumerate() {
        scheduled::for_each_statement_root(statement, |usage| {
            pending.push((usage, Consumer::Statement(index)))
        });
    }
    for value in raw.ir.blocks[block].terminator.operands() {
        pending.push((ValueUse::Reference(value), Consumer::Terminator));
    }
    while let Some((usage, owner)) = pending.pop() {
        let Some(remaining) = budget.checked_sub(1) else {
            return Vec::new();
        };
        *budget = remaining;
        if usage == ValueUse::Reference(before) {
            let eligible = match owner {
                Consumer::Value(value) => {
                    raw.ir.values[value].pc >= pc && !escaped.contains(&value)
                }
                Consumer::Statement(index) => raw.statement_pcs(block)[index] >= pc,
                Consumer::Terminator => true,
            };
            if eligible {
                targets.push(owner);
            }
        } else if usage.value() != next
            // An already recovered update owns its complete RHS semantics;
            // its operands are no longer independent source-expression sites.
            && !matches!(raw.ir.values[usage.value()].kind, ValueKind::LocalUpdate { .. })
        {
            usage.for_each_child(&raw.ir, |child| {
                pending.push((child, Consumer::Value(usage.value())))
            });
        }
    }
    targets
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Before,
    Pending,
    Done,
}

struct Relocation<'a> {
    old: LocalId,
    saved: LocalId,
    before: ValueId,
    next: ValueId,
    observed: bool,
    budget: &'a mut usize,
}

impl Relocation<'_> {
    fn effect(&mut self, effect: &Effect, phase: &mut Phase) -> bool {
        let Some(remaining) = self.budget.checked_sub(1) else {
            return false;
        };
        *self.budget = remaining;
        match effect {
            Effect::Read(local) => *local != self.saved || *phase == Phase::Done,
            Effect::Write { local, value, .. } if *local == self.saved => {
                // Later distinct updates may share this storage. This exact
                // relocated definition must still commit only once per path.
                if *phase == Phase::Before && *value == ValueUse::Reference(self.before) {
                    *phase = Phase::Pending;
                } else if *phase == Phase::Pending && *value == ValueUse::Definition(self.next) {
                    *phase = Phase::Done;
                } else if *phase != Phase::Done || *value == ValueUse::Definition(self.next) {
                    return false;
                }
                true
            }
            Effect::Write { local, .. } => *local != self.old || *phase != Phase::Pending,
            Effect::Throw => !self.observed || *phase != Phase::Pending,
            Effect::Sequence(effects) => effects.iter().all(|effect| self.effect(effect, phase)),
            Effect::Choice(left, right) => {
                let mut other = *phase;
                self.effect(left, phase) && self.effect(right, &mut other) && *phase == other
            }
            Effect::Empty => true,
        }
    }
}
