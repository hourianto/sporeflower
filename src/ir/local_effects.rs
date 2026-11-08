//! Ordered effects of scheduled expressions on physical source locals.
//! Reads stop at materialized values; a definition and InlineAssign RHS evaluate
//! their roots. Conditional arms remain alternatives, and a throwing operation
//! precedes the assignment that commits its result.
use super::execution::{Boundary, Evaluator, Fold, Operation};
use super::scheduled::ValueUse;
use super::{LocalId, MethodIr, Statement, Terminator, ValueKind};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub(crate) enum Effect {
    Empty,
    Read(LocalId),
    Write {
        local: LocalId,
        copied: Option<LocalId>,
        /// The evaluated RHS and its scheduled definition/read distinction.
        /// Value identities alone do not describe mutable local contents.
        value: ValueUse,
    },
    Throw,
    Sequence(Vec<Effect>),
    Choice(Box<Effect>, Box<Effect>),
}

impl Effect {
    fn sequence(effects: impl IntoIterator<Item = Self>) -> Self {
        let mut effects: Vec<_> = effects
            .into_iter()
            .filter(|effect| !matches!(effect, Self::Empty))
            .collect();
        match effects.len() {
            0 => Self::Empty,
            1 => effects.pop().unwrap(),
            _ => Self::Sequence(effects),
        }
    }

    pub(crate) fn reads_before_writes(
        &self,
        reads: &mut BTreeSet<LocalId>,
        defined: &mut BTreeSet<LocalId>,
    ) {
        match self {
            Self::Read(local) => {
                if !defined.contains(local) {
                    reads.insert(*local);
                }
            }
            Self::Write { local, .. } => {
                defined.insert(*local);
            }
            Self::Sequence(effects) => {
                for effect in effects {
                    effect.reads_before_writes(reads, defined);
                }
            }
            Self::Choice(left, right) => {
                let mut other = defined.clone();
                left.reads_before_writes(reads, defined);
                right.reads_before_writes(reads, &mut other);
                defined.retain(|local| other.contains(local));
            }
            Self::Empty | Self::Throw => {}
        }
    }

    pub(crate) fn writes(&self, visit: &mut impl FnMut(LocalId)) {
        self.assignments(&mut |local, _| visit(local));
    }

    pub(crate) fn assignments(&self, visit: &mut impl FnMut(LocalId, ValueUse)) {
        match self {
            Self::Write { local, value, .. } => visit(*local, *value),
            Self::Sequence(effects) => {
                for effect in effects {
                    effect.assignments(visit);
                }
            }
            Self::Choice(left, right) => {
                left.assignments(visit);
                right.assignments(visit);
            }
            _ => {}
        }
    }
}

pub(crate) fn expression(ir: &MethodIr, value: ValueUse) -> Effect {
    let mut events = Vec::new();
    Evaluator::new(ir, Effects(false), false).expression(value, None, &mut events);
    Effect::sequence(events)
}

pub(crate) fn statement(ir: &MethodIr, statement: &Statement, throwing: bool) -> Effect {
    let mut events = Vec::new();
    Evaluator::new(ir, Effects(throwing), throwing).statement(statement, &mut events);
    Effect::sequence(events)
}

pub(crate) fn terminator(ir: &MethodIr, terminal: &Terminator, throwing: bool) -> Effect {
    let mut events = Vec::new();
    Evaluator::new(ir, Effects(throwing), throwing).terminator(terminal, &mut events);
    Effect::sequence(events)
}

struct Effects(bool);

impl Fold for Effects {
    type Value = ();
    type Event = Effect;

    fn value(&mut self, operation: Operation<'_, ()>, events: &mut Vec<Effect>) {
        match operation {
            Operation::Read(_, local) => events.push(Effect::Read(local)),
            Operation::Target(..) => self.effect(Boundary::Throw, events),
            Operation::Previous(..) | Operation::Cycle(_) | Operation::Producer(..) => {}
        }
    }

    fn effect(&mut self, boundary: Boundary<(), Effect>, events: &mut Vec<Effect>) {
        match boundary {
            Boundary::Write(local, value, (), copied) => events.push(Effect::Write {
                local,
                value,
                copied,
            }),
            Boundary::Throw if self.0 => events.push(Effect::Throw),
            Boundary::Choice(_, _, [yes, no]) => events.push(Effect::Choice(
                Box::new(Effect::sequence(yes)),
                Box::new(Effect::sequence(no)),
            )),
            Boundary::Throw | Boundary::Index(()) => {}
        }
    }

    fn operation_throws(&self, ir: &MethodIr, usage: ValueUse) -> bool {
        // Once the array exists, initializer element stores are not separate
        // throwing operations in the storage-liveness model.
        !matches!(
            ir.values[usage.value()].kind,
            ValueKind::ArrayInitializer { .. }
        ) && ir.values[usage.value()].may_throw()
    }
}
