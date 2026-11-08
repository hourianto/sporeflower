//! One ordered walk owns bound expression occurrences, effects and size. Binding,
//! lifetime analysis and final scopes use the same operand and target roles.
use super::bound::{AssignmentValue, Body, Simple, Statement};
use crate::{
    ir::LocalId,
    java::syntax::{Expr, Expression, Operator},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Default)]
pub(super) struct Effects {
    pub(super) reads: BTreeMap<LocalId, usize>,
    pub(super) writes: BTreeMap<LocalId, usize>,
    heap_read: bool,
    heap_write: bool,
    pub(super) throws: bool,
    pub(super) statements: usize,
}

impl Effects {
    pub(super) fn occurrences(&self) -> impl Iterator<Item = (LocalId, usize)> + '_ {
        self.reads
            .iter()
            .chain(&self.writes)
            .map(|(&local, &count)| (local, count))
    }

    pub(super) fn discardable(&self) -> bool {
        self.writes.is_empty() && !self.heap_read && !self.heap_write && !self.throws
    }

    /// Exchanging evaluations must preserve both normal and abrupt completion.
    /// Heap operations are deliberately conservative; local dependencies remain
    /// precise, so unrelated declarations do not become artificial barriers.
    pub(super) fn commutes(&self, other: &Self) -> bool {
        if self
            .writes
            .keys()
            .any(|local| other.reads.contains_key(local) || other.writes.contains_key(local))
            || self
                .reads
                .keys()
                .any(|local| other.writes.contains_key(local))
        {
            return false;
        }
        let observable = self.heap_write || !self.writes.is_empty();
        let other_observable = other.heap_write || !other.writes.is_empty();
        !(self.throws && (other.throws || other_observable)
            || other.throws && observable
            || self.heap_read && other.heap_write
            || self.heap_write && (other.heap_read || other.heap_write))
    }
}

/// Heap evaluation can fail at the same boundary; a call may both read and write.
#[derive(Clone, Copy)]
pub(super) enum Operation {
    ReadHeap,
    WriteHeap,
    Call,
    Throw,
}

pub(super) trait Evaluation {
    fn statement(&mut self) {}
    fn enter(&mut self, _expression: &Expr) {}
    fn read(&mut self, _expression: &mut Expr, _local: LocalId, _conditional: bool) {}
    fn target_read(&mut self, expression: &mut Expr, local: LocalId, conditional: bool) {
        self.read(expression, local, conditional);
    }
    fn write(&mut self, _local: LocalId) {}
    fn operation(&mut self, _operation: Operation) {}
    fn mutation(&mut self) {}
    fn finish(&mut self, _expression: &mut Expr) {}
}

impl Evaluation for Effects {
    fn statement(&mut self) {
        self.statements += 1;
    }
    fn read(&mut self, _expression: &mut Expr, local: LocalId, _conditional: bool) {
        *self.reads.entry(local).or_default() += 1;
    }
    fn write(&mut self, local: LocalId) {
        *self.writes.entry(local).or_default() += 1;
    }
    fn operation(&mut self, operation: Operation) {
        self.heap_read |= matches!(operation, Operation::ReadHeap | Operation::Call);
        self.heap_write |= matches!(operation, Operation::WriteHeap | Operation::Call);
        self.throws = true;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Value,
    Address,
    ReadWrite,
}

/// Visit children in Java evaluation order. A target's address is distinct
/// from reading its value; its write completes after the right-hand operand.
fn evaluate(expression: &mut Expr, role: Role, conditional: bool, visitor: &mut impl Evaluation) {
    visitor.enter(expression);
    if let Expression::Local { id, .. } = expression.kind {
        match role {
            Role::Value => visitor.read(expression, id, conditional),
            Role::ReadWrite => visitor.target_read(expression, id, conditional),
            Role::Address => (),
        }
        visitor.finish(expression);
        return;
    }
    match &mut expression.kind {
        Expression::Atom(_)
        | Expression::Boolean(_)
        | Expression::Integer { .. }
        | Expression::Null => (),
        Expression::Local { .. } => unreachable!(),
        Expression::Group(value) | Expression::Commented(value, _) => {
            evaluate(value, role, conditional, visitor);
        }
        Expression::Field(value) => {
            evaluate(value, Role::Address, conditional, visitor);
            if role != Role::Address {
                visitor.operation(Operation::ReadHeap);
            }
        }
        Expression::Prefix(_, value) => evaluate(value, Role::Value, conditional, visitor),
        Expression::Cast {
            primitive, value, ..
        } => {
            evaluate(value, Role::Value, conditional, visitor);
            if !*primitive {
                visitor.operation(Operation::Throw);
            }
        }
        Expression::InstanceOf { value, .. } => evaluate(value, Role::Value, conditional, visitor),
        Expression::Update { target, .. } => {
            evaluate(target, Role::ReadWrite, conditional, visitor);
            write_target(target, visitor);
        }
        Expression::Binary(operator, operands) => {
            for operand in operands.iter_mut() {
                evaluate(operand, Role::Value, conditional, visitor);
            }
            if matches!(operator, Operator::Divide | Operator::Remainder) {
                visitor.operation(Operation::Throw);
            }
        }
        Expression::Logical { parts, .. } => {
            for (index, part) in parts.iter_mut().enumerate() {
                evaluate(part, Role::Value, conditional || index != 0, visitor);
            }
        }
        Expression::Member { receiver, .. } => {
            evaluate(receiver, Role::Value, conditional, visitor);
            // Member is also used for method/type selection. Actual field
            // reads carry Field, and invocation owns the eventual call effect.
        }
        Expression::Index(operands) => {
            for operand in operands.iter_mut() {
                evaluate(operand, Role::Value, conditional, visitor);
            }
            if role != Role::Address {
                visitor.operation(Operation::ReadHeap);
            }
        }
        Expression::Invocation { target, arguments } => {
            evaluate(target, Role::Address, conditional, visitor);
            for argument in arguments {
                evaluate(argument, Role::Value, conditional, visitor);
            }
            visitor.operation(Operation::Call);
        }
        Expression::New { arguments, .. } => {
            // Allocation/class initialization can fail before arguments run.
            visitor.operation(Operation::WriteHeap);
            for argument in arguments {
                evaluate(argument, Role::Value, conditional, visitor);
            }
            visitor.operation(Operation::Call);
        }
        Expression::NewArray { dimensions, .. } => {
            for dimension in dimensions {
                evaluate(dimension, Role::Value, conditional, visitor);
            }
            visitor.operation(Operation::WriteHeap);
        }
        Expression::Assignment { operands, compound } => {
            evaluate(
                &mut operands[0],
                if compound.is_some() {
                    Role::ReadWrite
                } else {
                    Role::Address
                },
                conditional,
                visitor,
            );
            evaluate(&mut operands[1], Role::Value, conditional, visitor);
            if matches!(compound, Some(Operator::Divide | Operator::Remainder)) {
                visitor.operation(Operation::Throw);
            }
            write_target(&operands[0], visitor);
        }
        Expression::Conditional(arms) => {
            evaluate(&mut arms[0], Role::Value, conditional, visitor);
            evaluate(&mut arms[1], Role::Value, true, visitor);
            evaluate(&mut arms[2], Role::Value, true, visitor);
        }
        Expression::Concatenation(parts) => {
            // String conversion can invoke user code between operands.
            for part in parts {
                evaluate(part, Role::Value, conditional, visitor);
                visitor.operation(Operation::Call);
            }
        }
        Expression::ArrayInitializer { elements, .. } => {
            visitor.operation(Operation::WriteHeap);
            for element in elements {
                evaluate(element, Role::Value, conditional, visitor);
            }
        }
    }
    visitor.finish(expression);
}

fn write_target(target: &Expr, visitor: &mut impl Evaluation) {
    visitor.mutation();
    match &target.kind {
        Expression::Local { id, .. } => visitor.write(*id),
        Expression::Group(target) | Expression::Commented(target, _) => {
            write_target(target, visitor)
        }
        _ => {
            visitor.operation(Operation::WriteHeap);
        }
    }
}

pub(super) fn effects(value: &mut Expr) -> Effects {
    let mut result = Effects::default();
    evaluate(value, Role::Value, false, &mut result);
    result
}

pub(in crate::java) fn expression_accesses(value: &mut Expr) -> BTreeSet<LocalId> {
    let result = effects(value);
    result.occurrences().map(|(local, _)| local).collect()
}

pub(super) fn expression_mutates(value: &mut Expr) -> bool {
    struct Mutation(bool);
    impl Evaluation for Mutation {
        fn mutation(&mut self) {
            self.0 = true;
        }
    }
    let mut result = Mutation(false);
    evaluate(value, Role::Value, false, &mut result);
    result.0
}

pub(super) fn expression_fits(value: &mut Expr) -> bool {
    let cost = cost(value);
    cost.nodes <= 80 && cost.operations <= 20 && cost.conditional_depth <= 3
}

pub(super) fn simple(value: &mut Simple, visitor: &mut impl Evaluation) {
    match value {
        Simple::Declare {
            binding,
            initialize,
        } => {
            if *initialize {
                visitor.write(binding.local);
            }
        }
        Simple::Assign { binding, value, .. } => {
            if !matches!(value, AssignmentValue::Value(_)) {
                evaluate(&mut binding.expression(), Role::ReadWrite, false, visitor);
            }
            match value {
                AssignmentValue::Value(value) | AssignmentValue::Compound { value, .. } => {
                    evaluate(value, Role::Value, false, visitor);
                }
                AssignmentValue::Increment { .. } => (),
            }
            if matches!(
                value,
                AssignmentValue::Compound {
                    operator: Operator::Divide | Operator::Remainder,
                    ..
                }
            ) {
                visitor.operation(Operation::Throw);
            }
            visitor.write(binding.local);
        }
        Simple::Expression(value) | Simple::Generated { value, .. } => {
            evaluate(value, Role::Value, false, visitor)
        }
    }
}

pub(in crate::java) fn simple_accesses(value: &mut Simple) -> BTreeSet<LocalId> {
    let mut result = Effects::default();
    simple(value, &mut result);
    result.occurrences().map(|(local, _)| local).collect()
}

/// Immediate statement evaluations only; nested bodies are separate movement
/// regions. Loop headers deliberately remain boundaries because they repeat.
pub(super) fn immediate(statement: &mut Statement, visitor: &mut impl Evaluation) {
    match statement {
        Statement::Simple(value) => simple(value, visitor),
        Statement::Return(Some(value)) => evaluate(value, Role::Value, false, visitor),
        Statement::Throw(value) => evaluate(&mut value.value, Role::Value, false, visitor),
        Statement::If { condition, .. } => evaluate(condition, Role::Value, false, visitor),
        Statement::Switch { value, .. } => evaluate(&mut value.value, Role::Value, false, visitor),
        Statement::Synchronized { monitor, .. } => evaluate(monitor, Role::Value, false, visitor),
        _ => (),
    }
}

pub(in crate::java) fn children(statement: &mut Statement, mut visit: impl FnMut(&mut Body)) {
    match statement {
        Statement::If { yes, no, .. } => {
            visit(yes);
            visit(no);
        }
        Statement::Loop { body, .. }
        | Statement::Block { body, .. }
        | Statement::Synchronized { body, .. } => visit(body),
        Statement::Switch { cases, .. } => {
            for case in cases {
                visit(&mut case.body);
            }
        }
        Statement::Try {
            body,
            catches,
            cleanup,
        } => {
            visit(body);
            for catch in catches {
                visit(&mut catch.body);
            }
            if let Some(cleanup) = cleanup {
                visit(cleanup);
            }
        }
        _ => (),
    }
}

pub(super) fn walk_body(body: &mut Body, result: &mut impl Evaluation) {
    for statement in body {
        walk_statement(statement, result);
    }
}

fn walk_statement(statement: &mut Statement, result: &mut impl Evaluation) {
    result.statement();
    evaluations(statement, result);
    if let Statement::Try { catches, .. } = statement {
        for catch in catches {
            if let Some(local) = catch.local {
                result.write(local);
            }
        }
    }
    children(statement, |body| walk_body(body, result));
}

pub(super) fn evaluate_header(header: &mut super::bound::LoopHeader, result: &mut impl Evaluation) {
    match header {
        super::LoopHeader::While(condition) | super::LoopHeader::DoWhile(condition) => {
            evaluate(condition, Role::Value, false, result);
        }
        super::LoopHeader::For {
            initializer,
            condition,
            update,
        } => {
            if let Some(initializer) = initializer {
                result.statement();
                simple(initializer, result);
            }
            evaluate(condition, Role::Value, false, result);
            result.statement();
            simple(update, result);
        }
    }
}

/// Binding and simplification share the complete source traversal, including
/// expressions in repeating headers and generated declarations.
pub(super) fn visit_expressions(body: &mut Body, visitor: impl FnMut(&mut Expr)) {
    walk_body(body, &mut Visit(visitor));
}

/// The same ordered traversal restricted to one statement's evaluations.
/// Exception scopes aggregate their children separately to subtract catches.
pub(super) fn visit_immediate_expressions(
    statement: &mut Statement,
    visitor: impl FnMut(&mut Expr),
) {
    evaluations(statement, &mut Visit(visitor));
}

/// Unlike movement's immediate region, this includes repeating loop headers.
pub(super) fn evaluations(statement: &mut Statement, visitor: &mut impl Evaluation) {
    immediate(statement, visitor);
    if let Statement::Loop { header, .. } = statement {
        evaluate_header(header, visitor);
    }
}

struct Visit<F>(F);
impl<F: FnMut(&mut Expr)> Evaluation for Visit<F> {
    fn finish(&mut self, expression: &mut Expr) {
        (self.0)(expression);
    }
}

#[derive(Default)]
pub(super) struct Cost {
    pub(super) nodes: usize,
    operations: usize,
    decisions: usize,
    conditional_depth: usize,
    conditions: usize,
}
impl Cost {
    pub(super) fn fits_into(&self, destination: &Self) -> bool {
        if self.nodes + destination.nodes > 80 {
            return false;
        }
        // Moving a result into an otherwise atomic consumer does not combine
        // computations. Combining arithmetic and decisions has a lower budget
        // than flat member/call chains, whose grouping remains readily visible.
        if destination.operations == 0 {
            return true;
        }
        let decisions = self.decisions + destination.decisions;
        self.operations + destination.operations <= if decisions == 0 { 20 } else { 12 }
            && decisions <= 3
    }
}
impl Evaluation for Cost {
    fn enter(&mut self, expression: &Expr) {
        if matches!(expression.kind, Expression::Conditional(_)) {
            self.conditions += 1;
            self.conditional_depth = self.conditional_depth.max(self.conditions);
        }
    }
    fn finish(&mut self, expression: &mut Expr) {
        self.nodes += 1;
        match &expression.kind {
            Expression::Binary(..) | Expression::Prefix(..) => self.operations += 1,
            Expression::Conditional(_) => {
                self.operations += 1;
                self.decisions += 1;
                self.conditions -= 1;
            }
            Expression::Logical { parts, .. } => {
                self.operations += parts.len().saturating_sub(1);
                self.decisions += parts.len().saturating_sub(1);
            }
            _ => (),
        }
    }
}
pub(super) fn cost(value: &mut Expr) -> Cost {
    let mut result = Cost::default();
    evaluate(value, Role::Value, false, &mut result);
    result
}

pub(super) fn statement_effects(statement: &mut Statement) -> Effects {
    let mut result = Effects::default();
    walk_statement(statement, &mut result);
    result
}

pub(super) fn add_counts(
    total: &mut BTreeMap<LocalId, usize>,
    added: impl IntoIterator<Item = (LocalId, usize)>,
) {
    for (local, count) in added {
        *total.entry(local).or_default() += count;
    }
}
