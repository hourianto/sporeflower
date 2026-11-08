//! Evaluate scheduled occurrences in Java operand order. Consumers choose the
//! result and event representation; storage boundaries, lazy arms and exception
//! positions have one implementation. Phi inputs are edges, never expressions.
use super::{
    CompareOp, LocalId, MethodIr, Statement, Terminator, UpdateTarget, ValueId, ValueKind, ValueUse,
};
use crate::classfile::{JvmType, MethodDescriptor};
use std::collections::HashSet;

/// Domain results are separate from execution boundaries: yielding the old
/// postfix value is not a second storage read in a local-effect analysis.
pub(crate) enum Operation<'a, V> {
    Read(ValueUse, LocalId),
    Previous(ValueUse, LocalId),
    Cycle(ValueUse),
    Producer(ValueUse, &'a [V]),
    /// Read and promote the target before the compound update's RHS.
    Target(ValueUse, &'a UpdateTarget, &'a [V]),
}

pub(crate) enum Boundary<V, E> {
    Write(LocalId, ValueUse, V, Option<LocalId>),
    Throw,
    Index(V),
    Choice(CompareOp, [V; 2], [Vec<E>; 2]),
}

pub(crate) trait Fold {
    type Value: Clone;
    type Event;
    fn value(
        &mut self,
        operation: Operation<'_, Self::Value>,
        events: &mut Vec<Self::Event>,
    ) -> Self::Value;
    fn effect(
        &mut self,
        boundary: Boundary<Self::Value, Self::Event>,
        events: &mut Vec<Self::Event>,
    );
    fn operation_throws(&self, ir: &MethodIr, usage: ValueUse) -> bool {
        ir.values[usage.value()].may_throw()
    }
}

pub(crate) struct Evaluator<'a, F: Fold> {
    ir: &'a MethodIr,
    pub(crate) fold: F,
    conversions: bool,
    visiting: HashSet<ValueUse>,
    inputs: Vec<F::Value>,
}

impl<'a, F: Fold> Evaluator<'a, F> {
    pub(crate) fn new(ir: &'a MethodIr, fold: F, conversions: bool) -> Self {
        Self {
            ir,
            fold,
            conversions,
            visiting: HashSet::new(),
            inputs: Vec::new(),
        }
    }

    pub(crate) fn expression(
        &mut self,
        usage: ValueUse,
        expected: Option<&JvmType>,
        events: &mut Vec<F::Event>,
    ) -> F::Value {
        let source = &self.ir.values[usage.value()];
        let local = usage.local(self.ir);
        let actual = local.map_or(&source.ty, |local| &self.ir.locals[local].ty);
        let conversion = self.conversions
            && expected.is_some_and(|expected| {
                expected.is_reference()
                    && actual.is_reference()
                    && expected != actual
                    && !matches!(expected, JvmType::Object(name) if name == "java/lang/Object")
                    && !(local.is_none()
                        && matches!(source.kind, ValueKind::Literal(super::Literal::Null)))
            });
        let result = if let Some(local) = local {
            self.fold.value(Operation::Read(usage, local), events)
        } else if !self.visiting.insert(usage) {
            return self.fold.value(Operation::Cycle(usage), events);
        } else {
            let start = self.inputs.len();
            match &source.kind {
                ValueKind::InlineAssign { local, value }
                | ValueKind::LocalUpdate { local, value, .. } => {
                    let before =
                        matches!(source.kind, ValueKind::LocalUpdate { postfix: true, .. }).then(
                            || {
                                self.fold.value(
                                    Operation::Previous(ValueUse::Reference(usage.value()), *local),
                                    events,
                                )
                            },
                        );
                    let rhs = ValueUse::Definition(*value);
                    let value = self.expression(rhs, Some(&self.ir.locals[*local].ty), events);
                    self.fold
                        .effect(Boundary::Write(*local, rhs, value.clone(), None), events);
                    self.inputs.push(before.unwrap_or(value));
                }
                ValueKind::Select {
                    condition,
                    then_value,
                    else_value,
                } => {
                    let left = self.reference(condition.left, None, events);
                    let right = self.reference(condition.right, None, events);
                    let mut yes = Vec::new();
                    let mut no = Vec::new();
                    let then_value = self.reference(*then_value, Some(&source.ty), &mut yes);
                    let else_value = self.reference(*else_value, Some(&source.ty), &mut no);
                    self.fold.effect(
                        Boundary::Choice(condition.op, [left.clone(), right.clone()], [yes, no]),
                        events,
                    );
                    self.inputs.extend([left, right, then_value, else_value]);
                }
                ValueKind::New {
                    descriptor, args, ..
                } => {
                    self.fold.effect(Boundary::Throw, events);
                    self.arguments(args, descriptor, events);
                }
                ValueKind::ArrayInitializer {
                    element_type,
                    values,
                    ..
                } => {
                    self.fold.effect(Boundary::Throw, events);
                    for &value in values {
                        self.operand(value, Some(element_type), events);
                    }
                }
                ValueKind::Call {
                    method,
                    receiver,
                    args,
                    ..
                } => {
                    self.receiver(receiver, &method.owner, events);
                    self.arguments(args, &method.descriptor, events);
                }
                ValueKind::Field { field, receiver } => {
                    self.receiver(receiver, &field.owner, events)
                }
                ValueKind::ArrayLoad {
                    array,
                    index,
                    element_type,
                } => self.array_target(*array, *index, element_type, events),
                ValueKind::Update { target, .. } | ValueKind::CompoundUpdate { target, .. } => {
                    match target {
                        UpdateTarget::Field { field, receiver } => {
                            self.receiver(receiver, &field.owner, events)
                        }
                        UpdateTarget::Array {
                            array,
                            index,
                            element_type,
                        } => self.array_target(*array, *index, element_type, events),
                    }
                    if let ValueKind::CompoundUpdate {
                        right,
                        operation_type,
                        ..
                    } = &source.kind
                    {
                        let old = self.fold.value(
                            Operation::Target(usage, target, &self.inputs[start..]),
                            events,
                        );
                        self.inputs.push(old);
                        self.operand(*right, Some(operation_type), events);
                    }
                }
                ValueKind::Phi { .. } => {}
                kind => kind.for_each_operand(|value| self.operand(value, None, events)),
            }
            if self.fold.operation_throws(self.ir, usage) {
                self.fold.effect(Boundary::Throw, events);
            }
            if matches!(
                source.kind,
                ValueKind::ArrayLoad { .. }
                    | ValueKind::Update {
                        target: UpdateTarget::Array { .. },
                        ..
                    }
            ) {
                self.fold
                    .effect(Boundary::Index(self.inputs[start + 1].clone()), events);
            }
            self.visiting.remove(&usage);
            let result = self
                .fold
                .value(Operation::Producer(usage, &self.inputs[start..]), events);
            self.inputs.truncate(start);
            result
        };
        if conversion {
            self.fold.effect(Boundary::Throw, events);
        }
        result
    }

    pub(crate) fn statement(
        &mut self,
        statement: &Statement,
        events: &mut Vec<F::Event>,
    ) -> Vec<F::Value> {
        let start = self.inputs.len();
        match statement {
            Statement::Assign { local, value } => {
                let node = self.expression(*value, Some(&self.ir.locals[*local].ty), events);
                let copied = value.local(self.ir).filter(|source| {
                    *source != *local && self.ir.locals[*source].ty == self.ir.locals[*local].ty
                });
                self.fold.effect(
                    Boundary::Write(*local, *value, node.clone(), copied),
                    events,
                );
                self.inputs.push(node);
            }
            Statement::Eval(value) => {
                let value = self.expression(ValueUse::Definition(*value), None, events);
                self.inputs.push(value);
            }
            Statement::PutField {
                field,
                receiver,
                value,
                ..
            } => {
                self.receiver(receiver, &field.owner, events);
                let expected = self
                    .conversions
                    .then(|| JvmType::parse(&field.descriptor).ok())
                    .flatten();
                self.operand(*value, expected.as_ref(), events);
            }
            Statement::ArrayStore {
                array,
                index,
                value,
                element_type,
            } => {
                self.array_target(*array, *index, element_type, events);
                self.operand(*value, Some(element_type), events);
            }
            Statement::ConstructorCall {
                method,
                receiver,
                args,
                ..
            } => {
                self.operand(*receiver, None, events);
                self.arguments(args, &method.descriptor, events);
            }
            Statement::Monitor { value, .. } => self.operand(*value, None, events),
        }
        if !matches!(statement, Statement::Assign { .. } | Statement::Eval(_)) {
            self.fold.effect(Boundary::Throw, events);
        }
        if matches!(statement, Statement::ArrayStore { .. }) {
            self.fold
                .effect(Boundary::Index(self.inputs[start + 1].clone()), events);
        }
        self.inputs.drain(start..).collect()
    }

    pub(crate) fn terminator(
        &mut self,
        terminator: &Terminator,
        events: &mut Vec<F::Event>,
    ) -> Vec<F::Value> {
        let expected = matches!(terminator, Terminator::Return(_)).then_some(&self.ir.return_type);
        let inputs = terminator
            .operands()
            .into_iter()
            .map(|value| self.reference(value, expected, events))
            .collect();
        if matches!(terminator, Terminator::Throw { .. }) {
            self.fold.effect(Boundary::Throw, events);
        }
        inputs
    }

    fn reference(
        &mut self,
        value: ValueId,
        expected: Option<&JvmType>,
        events: &mut Vec<F::Event>,
    ) -> F::Value {
        self.expression(ValueUse::Reference(value), expected, events)
    }

    fn operand(&mut self, value: ValueId, expected: Option<&JvmType>, events: &mut Vec<F::Event>) {
        let value = self.reference(value, expected, events);
        self.inputs.push(value);
    }

    fn arguments(&mut self, args: &[ValueId], descriptor: &str, events: &mut Vec<F::Event>) {
        let signature = self
            .conversions
            .then(|| MethodDescriptor::parse(descriptor).ok())
            .flatten();
        for (index, &value) in args.iter().enumerate() {
            self.operand(
                value,
                signature
                    .as_ref()
                    .and_then(|signature| signature.parameters.get(index)),
                events,
            );
        }
    }

    fn receiver(&mut self, receiver: &Option<ValueId>, owner: &str, events: &mut Vec<F::Event>) {
        if let Some(value) = receiver {
            let expected = self.conversions.then(|| JvmType::Object(owner.into()));
            self.operand(*value, expected.as_ref(), events);
        }
    }

    fn array_target(
        &mut self,
        array: ValueId,
        index: ValueId,
        element_type: &JvmType,
        events: &mut Vec<F::Event>,
    ) {
        let expected = self
            .conversions
            .then(|| JvmType::Array(Box::new(element_type.clone())));
        self.operand(array, expected.as_ref(), events);
        self.operand(index, Some(&JvmType::Int), events);
    }
}
