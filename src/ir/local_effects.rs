//! Ordered effects of scheduled expressions on physical source locals.
//! Reads stop at materialized values; a definition and InlineAssign RHS evaluate
//! their roots. Conditional arms remain alternatives, and a throwing operation
//! precedes the assignment that commits its result.
use super::scheduled::ValueUse;
use super::{LocalId, MethodIr, Statement, Terminator, ValueId, ValueKind};
use crate::classfile::{JvmType, MethodDescriptor};
use std::collections::{BTreeSet, HashSet};

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
    Builder {
        ir,
        throwing: false,
        visiting: HashSet::new(),
    }
    .value(value, None)
}

pub(crate) fn statement(ir: &MethodIr, statement: &Statement, throwing: bool) -> Effect {
    let mut builder = Builder {
        ir,
        throwing,
        visiting: HashSet::new(),
    };
    match statement {
        Statement::Assign { local, value } => {
            let source = value.local(ir);
            let copied = source.filter(|source| {
                *source != *local && ir.locals[*source].ty == ir.locals[*local].ty
            });
            Effect::sequence([
                builder.value(*value, Some(&ir.locals[*local].ty)),
                Effect::Write {
                    local: *local,
                    copied,
                    value: *value,
                },
            ])
        }
        Statement::Eval(value) => builder.value(ValueUse::Definition(*value), None),
        Statement::PutField {
            field,
            receiver,
            value,
            ..
        } => {
            let mut effects = Vec::new();
            if let Some(receiver) = receiver {
                effects.push(builder.value(
                    ValueUse::Reference(*receiver),
                    Some(&JvmType::Object(field.owner.clone())),
                ));
            }
            effects.push(builder.value(
                ValueUse::Reference(*value),
                JvmType::parse(&field.descriptor).ok().as_ref(),
            ));
            effects.push(builder.throw());
            Effect::sequence(effects)
        }
        Statement::ArrayStore {
            array,
            index,
            value,
            element_type,
        } => Effect::sequence([
            builder.value(
                ValueUse::Reference(*array),
                Some(&JvmType::Array(Box::new(element_type.clone()))),
            ),
            builder.value(ValueUse::Reference(*index), Some(&JvmType::Int)),
            builder.value(ValueUse::Reference(*value), Some(element_type)),
            builder.throw(),
        ]),
        Statement::ConstructorCall {
            method,
            receiver,
            args,
            ..
        } => {
            let mut effects = vec![builder.value(ValueUse::Reference(*receiver), None)];
            effects.extend(builder.arguments(args, &method.descriptor));
            effects.push(builder.throw());
            Effect::sequence(effects)
        }
        Statement::Monitor { value, .. } => Effect::sequence([
            builder.value(ValueUse::Reference(*value), None),
            builder.throw(),
        ]),
    }
}

pub(crate) fn terminator(ir: &MethodIr, terminal: &Terminator, throwing: bool) -> Effect {
    let mut builder = Builder {
        ir,
        throwing,
        visiting: HashSet::new(),
    };
    let mut effects: Vec<_> = terminal
        .operands()
        .into_iter()
        .map(|value| {
            builder.value(
                ValueUse::Reference(value),
                matches!(terminal, Terminator::Return(_)).then_some(&ir.return_type),
            )
        })
        .collect();
    if matches!(terminal, Terminator::Throw { .. }) {
        effects.push(builder.throw());
    }
    Effect::sequence(effects)
}

struct Builder<'a> {
    ir: &'a MethodIr,
    throwing: bool,
    visiting: HashSet<ValueUse>,
}

impl Builder<'_> {
    fn throw(&self) -> Effect {
        if self.throwing {
            Effect::Throw
        } else {
            Effect::Empty
        }
    }

    fn arguments(&mut self, values: &[ValueId], descriptor: &str) -> Vec<Effect> {
        let signature = self
            .throwing
            .then(|| MethodDescriptor::parse(descriptor).ok())
            .flatten();
        values
            .iter()
            .enumerate()
            .map(|(index, &value)| {
                self.value(
                    ValueUse::Reference(value),
                    signature
                        .as_ref()
                        .and_then(|signature| signature.parameters.get(index)),
                )
            })
            .collect()
    }

    fn value(&mut self, usage: ValueUse, expected: Option<&JvmType>) -> Effect {
        let id = usage.value();
        let value = &self.ir.values[id];
        let local = usage.local(self.ir);
        let actual = local.map_or(&value.ty, |local| &self.ir.locals[local].ty);
        let conversion_throws = self.throwing
            && expected.is_some_and(|expected| {
                expected.is_reference()
                    && actual.is_reference()
                    && expected != actual
                    && !matches!(expected, JvmType::Object(name) if name == "java/lang/Object")
                    && !(local.is_none()
                        && matches!(value.kind, ValueKind::Literal(super::Literal::Null)))
            });
        if let Some(local) = local {
            return if conversion_throws {
                Effect::sequence([Effect::Read(local), Effect::Throw])
            } else {
                Effect::Read(local)
            };
        }
        if !self.visiting.insert(usage) {
            return Effect::Empty;
        }
        let effect = match &value.kind {
            ValueKind::InlineAssign { local, value }
            | ValueKind::LocalUpdate { local, value, .. } => Effect::sequence([
                self.value(
                    ValueUse::Definition(*value),
                    Some(&self.ir.locals[*local].ty),
                ),
                Effect::Write {
                    local: *local,
                    copied: None,
                    value: ValueUse::Definition(*value),
                },
            ]),
            ValueKind::Select {
                condition,
                then_value,
                else_value,
            } => Effect::sequence([
                self.value(ValueUse::Reference(condition.left), None),
                self.value(ValueUse::Reference(condition.right), None),
                Effect::Choice(
                    Box::new(self.value(ValueUse::Reference(*then_value), Some(&value.ty))),
                    Box::new(self.value(ValueUse::Reference(*else_value), Some(&value.ty))),
                ),
            ]),
            ValueKind::New {
                descriptor, args, ..
            } => {
                // NEW can fail before evaluating an inline-captured argument;
                // the constructor can also fail after all arguments commit.
                let mut effects = vec![self.throw()];
                effects.extend(self.arguments(args, descriptor));
                effects.push(self.throw());
                Effect::sequence(effects)
            }
            ValueKind::ArrayInitializer {
                element_type,
                values,
                ..
            } => {
                let mut effects = vec![self.throw()];
                for &value in values {
                    effects.push(self.value(ValueUse::Reference(value), Some(element_type)));
                }
                Effect::sequence(effects)
            }
            ValueKind::Call {
                method,
                receiver,
                args,
                ..
            } => {
                let mut effects = Vec::new();
                if let Some(receiver) = receiver {
                    effects.push(self.value(
                        ValueUse::Reference(*receiver),
                        Some(&JvmType::Object(method.owner.clone())),
                    ));
                }
                effects.extend(self.arguments(args, &method.descriptor));
                effects.push(self.throw());
                Effect::sequence(effects)
            }
            ValueKind::ArrayLoad {
                array,
                index,
                element_type,
            } => Effect::sequence([
                self.value(
                    ValueUse::Reference(*array),
                    Some(&JvmType::Array(Box::new(element_type.clone()))),
                ),
                self.value(ValueUse::Reference(*index), Some(&JvmType::Int)),
                self.throw(),
            ]),
            ValueKind::Update { target, .. } | ValueKind::CompoundUpdate { target, .. } => {
                let mut effects = match target {
                    super::UpdateTarget::Field { field, receiver } => receiver
                        .iter()
                        .map(|&receiver| {
                            self.value(
                                ValueUse::Reference(receiver),
                                Some(&JvmType::Object(field.owner.clone())),
                            )
                        })
                        .collect::<Vec<_>>(),
                    super::UpdateTarget::Array {
                        array,
                        index,
                        element_type,
                    } => vec![
                        self.value(
                            ValueUse::Reference(*array),
                            Some(&JvmType::Array(Box::new(element_type.clone()))),
                        ),
                        self.value(ValueUse::Reference(*index), Some(&JvmType::Int)),
                    ],
                };
                effects.push(self.throw());
                if let ValueKind::CompoundUpdate {
                    right,
                    operation_type,
                    ..
                } = &value.kind
                {
                    effects.push(self.value(ValueUse::Reference(*right), Some(operation_type)));
                    effects.push(self.throw());
                }
                Effect::sequence(effects)
            }
            ValueKind::Field { field, receiver } => {
                let mut effects = Vec::new();
                if let Some(receiver) = receiver {
                    effects.push(self.value(
                        ValueUse::Reference(*receiver),
                        Some(&JvmType::Object(field.owner.clone())),
                    ));
                }
                effects.push(self.throw());
                Effect::sequence(effects)
            }
            ValueKind::Phi { .. } => Effect::Empty,
            _ => {
                let mut effects: Vec<_> = value
                    .kind
                    .operands()
                    .into_iter()
                    .map(|operand| self.value(ValueUse::Reference(operand), None))
                    .collect();
                if value.may_throw() {
                    effects.push(self.throw());
                }
                Effect::sequence(effects)
            }
        };
        self.visiting.remove(&usage);
        if conversion_throws {
            Effect::sequence([effect, Effect::Throw])
        } else {
            effect
        }
    }
}
