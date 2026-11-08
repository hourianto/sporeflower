//! Facts belong to a lexical execution path, not to a bytecode value number.
//! Completed reads also prove receiver non-nullness and permit repeated resolved
//! nonvolatile field tests. Scheduled writes and heap effects invalidate their
//! dependencies; joins keep only facts established by every completing arm.
use crate::ir::{
    BinaryOp, BlockId, CompareOp, Condition, Literal, LocalId, MethodIr, Terminator, ValueId,
    ValueKind, ValueUse, local_effects, scheduled,
};
use crate::structure::{Predicate, Stmt, SwitchCase, tree};
use crate::{java::expr::ExprPlanner, program::MemberId};
use std::collections::{BTreeMap, BTreeSet, HashSet};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Operand {
    Local(LocalId),
    Field(MemberId, Option<LocalId>),
    Int(i32),
    Null,
}

impl Operand {
    fn of(ir: &MethodIr, mut value: ValueId) -> Option<Self> {
        for _ in 0..ir.values.len() {
            if let Some(local) = ir.values[value].local {
                return Some(Self::Local(local));
            }
            match ir.values[value].kind {
                ValueKind::Alias(inner) => value = inner,
                ValueKind::Literal(Literal::Int(value)) => return Some(Self::Int(value)),
                ValueKind::Literal(Literal::Null) => return Some(Self::Null),
                _ => return None,
            }
        }
        None
    }

    fn source(expr: &ExprPlanner<'_, '_>, value: ValueId) -> Option<Self> {
        if let Some(operand) = Self::of(expr.ir, value) {
            return Some(operand);
        }
        match &expr.ir.values[value].kind {
            ValueKind::Alias(value) => Self::source(expr, *value),
            ValueKind::Field { field, receiver } => {
                let member =
                    expr.cx
                        .program
                        .resolve_field(&field.owner, &field.name, &field.descriptor)?;
                if expr.cx.program.field(member).access_flags & 0x0040 != 0 {
                    return None;
                }
                let receiver = match receiver {
                    Some(value) => match Self::of(expr.ir, *value)? {
                        Self::Local(local) => Some(local),
                        _ => return None,
                    },
                    None => None,
                };
                Some(Self::Field(member, receiver))
            }
            _ => None,
        }
    }

    fn depends_on(&self, local: LocalId) -> bool {
        matches!(self, Self::Local(value) | Self::Field(_, Some(value)) if *value == local)
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Test(Operand, u8, Operand);

#[derive(Clone, Copy)]
enum Integer {
    Known(i32),
    Nonzero,
    Unknown,
}

impl Integer {
    fn zero(self) -> Option<bool> {
        match self {
            Self::Known(value) => Some(value == 0),
            Self::Nonzero => Some(false),
            Self::Unknown => None,
        }
    }
}

fn test(ir: &MethodIr, head: BlockId, negated: bool) -> Option<(Test, bool)> {
    let Terminator::Branch { condition, .. } = &ir.blocks[head].terminator else {
        return None;
    };
    condition_test(ir, condition, negated)
}

fn condition_test(ir: &MethodIr, condition: &Condition, negated: bool) -> Option<(Test, bool)> {
    Some(normalize_test(
        Operand::of(ir, condition.left)?,
        condition.op,
        Operand::of(ir, condition.right)?,
        negated,
    ))
}

fn source_test(
    expr: &ExprPlanner<'_, '_>,
    condition: &Condition,
    negated: bool,
) -> Option<(Test, bool)> {
    let left = Operand::source(expr, condition.left)?;
    let right = Operand::source(expr, condition.right)?;
    // The second static read may initialize a class which changes the first
    // field after its value was captured. Keep relations to stable locals or
    // constants; a pair of mutable reads needs an ordered snapshot proof.
    if matches!((&left, &right), (Operand::Field(..), Operand::Field(..))) {
        return None;
    }
    Some(normalize_test(left, condition.op, right, negated))
}

fn normalize_test(
    mut left: Operand,
    mut op: CompareOp,
    mut right: Operand,
    negated: bool,
) -> (Test, bool) {
    if right < left {
        std::mem::swap(&mut left, &mut right);
        op = op.reversed();
    }
    let (kind, inverse) = match op {
        CompareOp::Eq => (0, false),
        CompareOp::Ne => (0, true),
        CompareOp::Lt => (1, false),
        CompareOp::Ge => (1, true),
        CompareOp::Le => (2, false),
        CompareOp::Gt => (2, true),
    };
    (Test(left, kind, right), inverse ^ negated)
}

/// Compare scheduled local/constant tests, independently of bytecode identity.
/// Both alternatives read the same current locals and perform no operation
/// whose evaluation could distinguish their otherwise identical outcomes.
pub(in crate::java) fn same_local_predicate(
    ir: &MethodIr,
    left: &Predicate,
    right: &Predicate,
) -> bool {
    left.equivalent_by(right, &mut |a, an, b, bn| {
        test(ir, a, an).is_some_and(|a| test(ir, b, bn) == Some(a))
    })
}

#[derive(Clone, Default)]
pub(super) struct Facts {
    tests: BTreeMap<Test, bool>,
    initialized: BTreeSet<usize>,
}

impl Facts {
    pub fn path(&self, expr: &ExprPlanner<'_, '_>, condition: &Condition, truth: bool) -> Self {
        let mut path = self.clone();
        path.effects(
            expr,
            vec![
                ValueUse::Reference(condition.left),
                ValueUse::Reference(condition.right),
            ],
            local_effects::Effect::Sequence(vec![
                local_effects::expression(expr.ir, ValueUse::Reference(condition.left)),
                local_effects::expression(expr.ir, ValueUse::Reference(condition.right)),
            ]),
            true,
        );
        if let Some((test, inverse)) = source_test(expr, condition, false) {
            path.tests.insert(test, truth ^ inverse);
        }
        path
    }

    pub fn bind_predicate(
        &self,
        expr: &ExprPlanner<'_, '_>,
        predicate: &Predicate,
        negated: bool,
    ) -> crate::Result<crate::java::syntax::Expr> {
        if let Predicate::Atom {
            head,
            negated: inverse,
        } = predicate
            && let Terminator::Branch { condition, .. } = &expr.ir.blocks[*head].terminator
        {
            return self.bind_condition(expr, condition, negated ^ inverse, false);
        }
        expr.predicate(&self.simplify(expr, predicate), negated, false)
    }

    pub fn bind_condition(
        &self,
        expr: &ExprPlanner<'_, '_>,
        condition: &Condition,
        negated: bool,
        as_value: bool,
    ) -> crate::Result<crate::java::syntax::Expr> {
        use crate::{
            classfile::JvmType,
            java::syntax::{Expr, Prefix},
        };
        if let Some(truth) = self.condition(expr, condition) {
            return Ok(Expr::boolean(truth ^ negated));
        }
        if matches!(condition.op, CompareOp::Eq | CompareOp::Ne)
            && expr.ir.values[condition.left].ty == JvmType::Boolean
            && matches!(Operand::of(expr.ir, condition.right), Some(Operand::Int(0)))
            && let Some(value) = self.mask(expr, condition.left, 0)?
        {
            return Ok(if (condition.op == CompareOp::Eq) ^ negated {
                Expr::prefix(Prefix::Not, value)
            } else if as_value && !expr.canonical_boolean(ValueUse::Reference(condition.left)) {
                value.truth_value()
            } else {
                value
            });
        }
        expr.condition_expression(condition, negated, as_value)
    }

    /// Fold only exact 0/1 factors in an eager mask. Nonzero raw slots do not
    /// establish their low bit, and a leading 1 still masks an unknown operand.
    fn mask(
        &self,
        expr: &ExprPlanner<'_, '_>,
        value: ValueId,
        depth: usize,
    ) -> crate::Result<Option<crate::java::syntax::Expr>> {
        use crate::java::syntax::Expr;
        if depth >= 512 {
            return Ok(None);
        }
        if self.discardable(expr, value)
            && let Integer::Known(value @ (0 | 1)) = self.integer(expr, value)
        {
            return Ok(Some(Expr::boolean(value != 0)));
        }
        if expr.ir.values[value].local.is_none()
            && let ValueKind::Binary {
                op: BinaryOp::And,
                left,
                right,
            } = expr.ir.values[value].kind
        {
            let first = self.mask(expr, left, depth + 1)?;
            let mut after_left = self.clone();
            after_left.effects(
                expr,
                vec![scheduled::ValueUse::Reference(left)],
                local_effects::expression(expr.ir, ValueUse::Reference(left)),
                true,
            );
            let second = after_left.mask(expr, right, depth + 1)?;
            if first.is_some() || second.is_some() {
                return Ok(Some(expr.boolean_binary(
                    BinaryOp::And,
                    [left, right],
                    [
                        match first {
                            Some(value) => value,
                            None => {
                                expr.source_value(ValueUse::Reference(left), super::Context::Test)?
                            }
                        },
                        match second {
                            Some(value) => value,
                            None => {
                                expr.source_value(ValueUse::Reference(right), super::Context::Test)?
                            }
                        },
                    ],
                )));
            }
        }
        Ok(None)
    }

    pub fn known(&self, expr: &ExprPlanner<'_, '_>, predicate: &Predicate) -> Option<bool> {
        let ir = expr.ir;
        match predicate {
            Predicate::Atom { head, negated } => {
                let Terminator::Branch { condition, .. } = &ir.blocks[*head].terminator else {
                    return None;
                };
                self.condition(expr, condition).map(|value| value ^ negated)
            }
            Predicate::And(terms) | Predicate::Or(terms) => {
                let conjunction = matches!(predicate, Predicate::And(_));
                for term in terms {
                    // Unknown prefixes can have effects: a known later false
                    // term does not make the entire evaluation removable.
                    if self.known(expr, term)? != conjunction {
                        return Some(!conjunction);
                    }
                }
                Some(conjunction)
            }
            Predicate::Select { condition, yes, no } => self.known(
                expr,
                if self.known(expr, condition)? {
                    yes
                } else {
                    no
                },
            ),
        }
    }

    /// Remove neutral tests on the path where each short-circuit operand runs.
    /// A determining constant after an unknown prefix must keep that prefix's
    /// evaluation; only a known neutral operand can disappear independently.
    pub fn simplify(&self, expr: &ExprPlanner<'_, '_>, predicate: &Predicate) -> Predicate {
        match predicate {
            Predicate::Atom { .. } => predicate.clone(),
            Predicate::And(terms) | Predicate::Or(terms) => {
                let conjunction = matches!(predicate, Predicate::And(_));
                let mut path = self.clone();
                let mut kept = Vec::new();
                for term in terms {
                    let term = path.simplify(expr, term);
                    match path.known(expr, &term) {
                        Some(truth) if truth == conjunction => continue,
                        Some(_) => {
                            kept.push(term);
                            break;
                        }
                        None => (),
                    }
                    path.predicate(expr, &term);
                    path.assume(expr, &term, conjunction);
                    kept.push(term);
                }
                match kept.len() {
                    // The caller's ordinary constant-predicate check owns the
                    // literal result; Predicate itself contains only tests.
                    0 => predicate.clone(),
                    1 => kept.pop().unwrap(),
                    _ if conjunction => Predicate::And(kept),
                    _ => Predicate::Or(kept),
                }
            }
            Predicate::Select { condition, yes, no } => {
                if let Some(truth) = self.known(expr, condition) {
                    return self.simplify(expr, if truth { yes } else { no });
                }
                let condition = self.simplify(expr, condition);
                let mut path = self.clone();
                path.predicate(expr, &condition);
                let mut selected = path.clone();
                selected.assume(expr, &condition, true);
                path.assume(expr, &condition, false);
                Predicate::Select {
                    condition: Box::new(condition),
                    yes: Box::new(selected.simplify(expr, yes)),
                    no: Box::new(path.simplify(expr, no)),
                }
            }
        }
    }

    /// A direct switch entry excludes every label belonging to another arm.
    /// The caller joins this with any preceding arm that can fall through.
    pub fn switch_arm(
        &self,
        expr: &ExprPlanner<'_, '_>,
        head: BlockId,
        cases: &[SwitchCase],
        selected: usize,
    ) -> Self {
        let mut path = self.clone();
        let terminal = &expr.ir.blocks[head].terminator;
        path.terminal(expr, terminal, true);
        let Terminator::Switch { value, .. } = terminal else {
            return path;
        };
        let Some(operand) = Operand::source(expr, *value) else {
            return path;
        };
        // Bulk-build the exclusions: large dispatch switches otherwise perform
        // a tree insertion for every key in every arm.
        let mut excluded = cases
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != selected)
            .flat_map(|(_, case)| &case.keys)
            .map(|key| normalize_test(operand.clone(), CompareOp::Eq, Operand::Int(*key), false))
            .collect::<BTreeMap<_, _>>();
        path.tests.append(&mut excluded);
        if !cases[selected].default
            && let [key] = cases[selected].keys.as_slice()
        {
            let (test, inverse) = normalize_test(operand, CompareOp::Eq, Operand::Int(*key), false);
            path.tests.insert(test, !inverse);
        }
        path
    }

    /// Select an entire arm only when the selector read itself can disappear.
    /// This also handles a default whose explicit alternatives are all excluded.
    pub fn selected_case(
        &self,
        expr: &ExprPlanner<'_, '_>,
        value: ValueId,
        cases: &[SwitchCase],
    ) -> Option<usize> {
        if !self.discardable(expr, value) {
            return None;
        }
        let operand = Operand::source(expr, value)?;
        let exact = match operand {
            Operand::Int(value) => Some(value),
            _ => self.tests.iter().find_map(|(test, truth)| match test {
                Test(left, 0, Operand::Int(value)) if *truth && *left == operand => Some(*value),
                _ => None,
            }),
        };
        let mut unknown = false;
        for (index, case) in cases.iter().enumerate() {
            for key in &case.keys {
                let (test, inverse) =
                    normalize_test(operand.clone(), CompareOp::Eq, Operand::Int(*key), false);
                let truth = exact
                    .map(|value| value == *key)
                    .or_else(|| self.tests.get(&test).map(|truth| truth ^ inverse));
                match truth {
                    Some(true) => return Some(index),
                    Some(false) => (),
                    None => unknown = true,
                }
            }
        }
        (!unknown)
            .then(|| cases.iter().position(|case| case.default))
            .flatten()
    }

    pub fn condition(&self, expr: &ExprPlanner<'_, '_>, condition: &Condition) -> Option<bool> {
        let ir = expr.ir;
        if !self.discardable(expr, condition.left) || !self.discardable(expr, condition.right) {
            return None;
        }
        if let Some((test, inverse)) = source_test(expr, condition, false)
            && let Some(value) = self.tests.get(&test)
        {
            return Some(value ^ inverse);
        }
        if matches!(condition.op, CompareOp::Eq | CompareOp::Ne) {
            let value = if matches!(Operand::of(ir, condition.right), Some(Operand::Int(0))) {
                condition.left
            } else if matches!(Operand::of(ir, condition.left), Some(Operand::Int(0))) {
                condition.right
            } else {
                return None;
            };
            return self
                .integer(expr, value)
                .zero()
                .map(|zero| zero == (condition.op == CompareOp::Eq));
        }
        None
    }

    fn integer(&self, expr: &ExprPlanner<'_, '_>, value: ValueId) -> Integer {
        let ir = expr.ir;
        if let Some(local) = ir.values[value].local {
            return match self
                .tests
                .get(&Test(Operand::Local(local), 0, Operand::Int(0)))
            {
                Some(true) => Integer::Known(0),
                Some(false) if expr.canonical_boolean(ValueUse::Reference(value)) => {
                    Integer::Known(1)
                }
                Some(false) => Integer::Nonzero,
                None => Integer::Unknown,
            };
        }
        match &ir.values[value].kind {
            ValueKind::Literal(Literal::Int(value)) => Integer::Known(*value),
            ValueKind::Alias(value) => self.integer(expr, *value),
            ValueKind::Binary {
                op: op @ (BinaryOp::And | BinaryOp::Or),
                left,
                right,
            } => match (op, self.integer(expr, *left), self.integer(expr, *right)) {
                (BinaryOp::And, Integer::Known(a), Integer::Known(b)) => Integer::Known(a & b),
                (BinaryOp::Or, Integer::Known(a), Integer::Known(b)) => Integer::Known(a | b),
                (BinaryOp::And, a, b) if a.zero() == Some(true) || b.zero() == Some(true) => {
                    Integer::Known(0)
                }
                (BinaryOp::Or, a, b) if a.zero() == Some(false) || b.zero() == Some(false) => {
                    Integer::Nonzero
                }
                _ => Integer::Unknown,
            },
            ValueKind::Select {
                condition,
                then_value,
                else_value,
            } => match self.condition(expr, condition) {
                Some(truth) => self.integer(expr, if truth { *then_value } else { *else_value }),
                None => Integer::Unknown,
            },
            _ => Integer::Unknown,
        }
    }

    pub fn assume(&mut self, expr: &ExprPlanner<'_, '_>, predicate: &Predicate, truth: bool) {
        let ir = expr.ir;
        match predicate {
            Predicate::Atom { head, negated } => {
                let Terminator::Branch { condition, .. } = &ir.blocks[*head].terminator else {
                    return;
                };
                if let Some((test, inverse)) = source_test(expr, condition, *negated) {
                    self.tests.insert(test, truth ^ inverse);
                }
            }
            Predicate::And(terms) if truth => {
                for term in terms {
                    self.predicate(expr, term);
                    self.assume(expr, term, true);
                }
            }
            Predicate::Or(terms) if !truth => {
                for term in terms {
                    self.predicate(expr, term);
                    self.assume(expr, term, false);
                }
            }
            _ => (),
        }
    }

    pub fn invalidate(&mut self, effect: local_effects::Effect) {
        effect.writes(&mut |local| {
            self.tests.retain(|Test(left, _, right), _| {
                !left.depends_on(local) && !right.depends_on(local)
            });
        });
    }

    pub fn predicate(&mut self, expr: &ExprPlanner<'_, '_>, predicate: &Predicate) {
        self.predicate_effects(expr, predicate, true);
    }

    fn predicate_effects(
        &mut self,
        expr: &ExprPlanner<'_, '_>,
        predicate: &Predicate,
        remember: bool,
    ) {
        let ir = expr.ir;
        // Compound predicates can skip a later read or overwrite an earlier
        // receiver. Their guaranteed arms are transferred in order by assume.
        if let Predicate::Atom { head, .. } = predicate {
            self.terminal(expr, &ir.blocks[*head].terminator, remember);
        } else {
            for head in predicate.heads() {
                self.terminal(expr, &ir.blocks[head].terminator, false);
            }
        }
    }

    pub fn join(mut self, other: &Self) -> Self {
        self.tests
            .retain(|test, truth| other.tests.get(test) == Some(truth));
        self.initialized
            .retain(|class| other.initialized.contains(class));
        self
    }

    pub fn forget_heap(&mut self) {
        self.tests.retain(|Test(left, _, right), _| {
            !matches!(left, Operand::Field(..)) && !matches!(right, Operand::Field(..))
        });
    }

    fn nonnull(&self, ir: &MethodIr, value: ValueId) -> bool {
        ir.is_this(value)
            || Operand::of(ir, value).is_some_and(|operand| {
                self.tests.get(&Test(operand, 0, Operand::Null)) == Some(&false)
            })
    }

    fn discardable(&self, expr: &ExprPlanner<'_, '_>, value: ValueId) -> bool {
        let ir = expr.ir;
        scheduled::values(ir, scheduled::ValueUse::Reference(value))
            .operations()
            .all(|value| {
                if !ir.operation_is_observable(value) {
                    return true;
                }
                match &ir.values[value].kind {
                    ValueKind::Field { receiver, .. } => match Operand::source(expr, value) {
                        Some(Operand::Field(member, _)) => receiver.map_or_else(
                            || self.initialized.contains(&member.class),
                            |receiver| self.nonnull(ir, receiver),
                        ),
                        _ => false,
                    },
                    ValueKind::ArrayLength { array } => self.nonnull(ir, *array),
                    _ => false,
                }
            })
    }

    /// The inventory includes both selected arms when killing facts. Only
    /// unconditional operations establish facts on normal completion. A local
    /// changed anywhere in the evaluation cannot name a previously read object.
    fn effects(
        &mut self,
        expr: &ExprPlanner<'_, '_>,
        roots: Vec<scheduled::ValueUse>,
        effect: local_effects::Effect,
        remember: bool,
    ) {
        let ir = expr.ir;
        let mut written = BTreeSet::new();
        effect.writes(&mut |local| {
            written.insert(local);
        });
        self.invalidate(effect);
        let mut pending: Vec<_> = roots.into_iter().map(|root| (root, remember)).collect();
        let mut seen = HashSet::new();
        let mut initialized = BTreeSet::new();
        while let Some((usage, guaranteed)) = pending.pop() {
            if !seen.insert((usage, guaranteed)) || usage.local(ir).is_some() {
                continue;
            }
            let kind = &ir.values[usage.value()].kind;
            // A possible arm can invalidate facts, but cannot establish them.
            // Keep children conditional without relying on visitation order.
            usage.for_each_child(ir, |child| {
                pending.push((
                    child,
                    guaranteed && !matches!(kind, ValueKind::Select { .. }),
                ))
            });
            match kind {
                ValueKind::Call { .. }
                | ValueKind::New { .. }
                | ValueKind::Allocation { .. }
                | ValueKind::InitializeClass { .. }
                | ValueKind::CompoundUpdate { .. }
                | ValueKind::Update { .. } => self.forget_heap(),
                ValueKind::Field { field, receiver } => {
                    let member =
                        expr.cx
                            .program
                            .resolve_field(&field.owner, &field.name, &field.descriptor);
                    if member.is_none_or(|member| {
                        expr.cx.program.field(member).access_flags & 0x0040 != 0
                            || receiver.is_none() && !self.initialized.contains(&member.class)
                    }) {
                        self.forget_heap();
                    }
                    if guaranteed
                        && receiver.is_none()
                        && let Some(member) = member
                    {
                        initialized.insert(member.class);
                    }
                }
                _ => (),
            }
            let receiver = match kind {
                ValueKind::Call { receiver, .. } | ValueKind::Field { receiver, .. } => *receiver,
                ValueKind::ArrayLoad { array, .. } | ValueKind::ArrayLength { array } => {
                    Some(*array)
                }
                _ => None,
            };
            if guaranteed
                && let Some(receiver) = receiver
                && let Some(Operand::Local(local)) = Operand::of(ir, receiver)
                && !written.contains(&local)
            {
                self.tests
                    .insert(Test(Operand::Local(local), 0, Operand::Null), false);
            }
        }
        self.initialized.extend(initialized);
    }

    fn terminal(&mut self, expr: &ExprPlanner<'_, '_>, terminal: &Terminator, remember: bool) {
        self.effects(
            expr,
            terminal
                .operands()
                .into_iter()
                .map(scheduled::ValueUse::Reference)
                .collect(),
            local_effects::terminator(expr.ir, terminal, false),
            remember,
        );
    }

    pub fn statement(
        &mut self,
        expr: &ExprPlanner<'_, '_>,
        statement: &crate::ir::Statement,
        remember: bool,
    ) {
        use crate::ir::Statement;
        if matches!(
            statement,
            Statement::PutField { .. }
                | Statement::ConstructorCall { .. }
                | Statement::Monitor { .. }
        ) {
            self.forget_heap();
        }
        let mut roots = Vec::new();
        scheduled::for_each_statement_root(statement, |root| roots.push(root));
        self.effects(
            expr,
            roots,
            local_effects::statement(expr.ir, statement, false),
            remember,
        );
    }

    pub fn without_writes(mut self, expr: &ExprPlanner<'_, '_>, body: &[Stmt]) -> Self {
        let ir = expr.ir;
        for statement in tree::walk(body) {
            if self.tests.is_empty() {
                return self;
            }
            match statement {
                Stmt::Dispatch { .. } | Stmt::BoundRegion { .. } => return Self::default(),
                Stmt::Synchronized { .. } => self.forget_heap(),
                _ => (),
            }
            statement.operations(ir, &mut |operation| match operation {
                tree::Operation::Statement { statement, .. } => {
                    self.statement(expr, statement, false)
                }
                tree::Operation::Terminator { terminator, .. } => {
                    self.terminal(expr, terminator, false)
                }
                tree::Operation::Expression(value) => {
                    self.invalidate(local_effects::expression(ir, ValueUse::Reference(value)));
                }
                tree::Operation::CatchBinding(value) => {
                    if let Some(local) = ir.values[value].local {
                        self.tests.retain(|Test(left, _, right), _| {
                            !left.depends_on(local) && !right.depends_on(local)
                        });
                    }
                }
            });
        }
        self
    }
}
