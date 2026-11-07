use super::Analysis;
use crate::index::{BlockId, LocalId, ValueId};
use crate::ir::{Literal, MethodIr, Statement, Terminator, UpdateTarget, ValueKind};
use crate::structure::bindings::{Scope, Scopes};
use crate::structure::tree::{self, Binder};
use crate::structure::{Predicate, Stmt};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn same(analysis: &Analysis<'_>, left: &[Stmt], right: &[Stmt]) -> bool {
    same_at(analysis, left, right, None)
}

pub(super) fn same_at(
    analysis: &Analysis<'_>,
    left: &[Stmt],
    right: &[Stmt],
    entry: Option<(BlockId, usize)>,
) -> bool {
    let ir = analysis.ir;
    let mut writes = written_locals(ir, left);
    writes.extend(written_locals(ir, right));
    Matcher {
        ir,
        available: Some(analysis),
        point: None,
        entry,
        writes,
        left: sites(ir, left),
        right: sites(ir, right),
        locals: BTreeMap::new(),
        bindings: Scopes::default(),
        scope: None,
    }
    .body(left, right)
}

pub(super) fn same_exit(ir: &MethodIr, left: &Stmt, right: &Stmt) -> bool {
    match (left, right) {
        (Stmt::Return(a), Stmt::Return(b)) => {
            let (Terminator::Return(a), Terminator::Return(b)) =
                (&ir.blocks[*a].terminator, &ir.blocks[*b].terminator)
            else {
                return false;
            };
            match (a, b) {
                (None, None) => true,
                (Some(a), Some(b)) => Matcher {
                    ir,
                    available: None,
                    point: None,
                    entry: None,
                    writes: BTreeSet::new(),
                    left: BTreeSet::new(),
                    right: BTreeSet::new(),
                    locals: BTreeMap::new(),
                    bindings: Scopes::default(),
                    scope: None,
                }
                .value(*a, *b, false),
                _ => false,
            }
        }
        (Stmt::Break { target: a }, Stmt::Break { target: b }) => a == b,
        (Stmt::Continue { header: a }, Stmt::Continue { header: b }) => a == b,
        _ => false,
    }
}

#[derive(Clone)]
struct Matcher<'a> {
    ir: &'a MethodIr,
    available: Option<&'a Analysis<'a>>,
    point: Option<(BlockId, usize)>,
    entry: Option<(BlockId, usize)>,
    writes: BTreeSet<LocalId>,
    left: BTreeSet<(BlockId, usize)>,
    right: BTreeSet<(BlockId, usize)>,
    locals: BTreeMap<LocalId, LocalId>,
    bindings: Scopes<(Binder, Binder)>,
    scope: Scope,
}

impl Matcher<'_> {
    fn local(&mut self, a: LocalId, b: LocalId) -> bool {
        if a == b {
            return true;
        }
        if let Some(mapped) = self.locals.get(&a) {
            return *mapped == b;
        }
        if self.locals.values().any(|mapped| *mapped == b)
            || self.ir.locals[a].ty != self.ir.locals[b].ty
            || !internal(self.ir, a, &self.left)
            || !internal(self.ir, b, &self.right)
        {
            return false;
        }
        self.locals.insert(a, b);
        true
    }

    fn read_local(&mut self, left: LocalId, right: LocalId) -> bool {
        if self.local(left, right) {
            return true;
        }
        let Some((block, index)) = self.point else {
            return false;
        };
        let Some(available) = self.available else {
            return false;
        };
        let left = self.locals.get(&left).copied().unwrap_or(left);
        let available = available.available();
        available.reads_same(block, index, left, right)
            || self.entry.is_some_and(|(block, index)| {
                !self.writes.contains(&left)
                    && !self.writes.contains(&right)
                    && available.reads_same(block, index, left, right)
            })
    }

    fn read_literal(&self, local: LocalId, literal: &Literal) -> bool {
        let Some((block, index)) = self.point else {
            return false;
        };
        self.available.is_some_and(|available| {
            let available = available.available();
            available.reads_literal(block, index, local, literal)
                || self.entry.is_some_and(|(block, index)| {
                    !self.writes.contains(&local)
                        && available.reads_literal(block, index, local, literal)
                })
        })
    }

    fn value(&mut self, a: ValueId, b: ValueId, root: bool) -> bool {
        if a == b {
            if self.point.is_some() && inline_write(self.ir, a, root) {
                self.point = None;
            }
            return true;
        }
        if !root {
            match (self.ir.values[a].local, self.ir.values[b].local) {
                (Some(a), Some(b)) => return self.read_local(a, b),
                (Some(local), None) => {
                    let ValueKind::Literal(literal) = &self.ir.values[b].kind else {
                        return false;
                    };
                    return self
                        .read_literal(self.locals.get(&local).copied().unwrap_or(local), literal);
                }
                (None, Some(local)) => {
                    let ValueKind::Literal(literal) = &self.ir.values[a].kind else {
                        return false;
                    };
                    return self.read_literal(local, literal);
                }
                _ => (),
            }
        }
        let a = &self.ir.values[a].kind;
        let b = &self.ir.values[b].kind;
        let attributes = match (a, b) {
            (ValueKind::InitializeClass { class: a }, ValueKind::InitializeClass { class: b }) => {
                a == b
            }
            (ValueKind::Alias(a), ValueKind::Alias(b)) => return self.value(*a, *b, false),
            (
                ValueKind::InlineAssign { local: a, .. },
                ValueKind::InlineAssign { local: b, .. },
            ) => {
                // Block facts describe the point before this expression. Once
                // an embedded assignment executes, later operands need fresh
                // facts; keep their comparison strict until the next statement.
                self.point = None;
                self.local(*a, *b)
            }
            (
                ValueKind::LocalUpdate {
                    local: a,
                    increment: x,
                    postfix: p,
                    ..
                },
                ValueKind::LocalUpdate {
                    local: b,
                    increment: y,
                    postfix: q,
                    ..
                },
            ) => {
                self.point = None;
                x == y && p == q && self.local(*a, *b)
            }
            (ValueKind::Select { condition: a, .. }, ValueKind::Select { condition: b, .. }) => {
                a.op == b.op
            }
            (ValueKind::Parameter { slot: a, .. }, ValueKind::Parameter { slot: b, .. }) => a == b,
            (ValueKind::Literal(a), ValueKind::Literal(b)) => literal(a, b),
            (ValueKind::Unary { op: a, .. }, ValueKind::Unary { op: b, .. }) => a == b,
            (ValueKind::Binary { op: a, .. }, ValueKind::Binary { op: b, .. }) => a == b,
            (ValueKind::Compare { kind: a, .. }, ValueKind::Compare { kind: b, .. }) => a == b,
            (ValueKind::Cast { ty: a, .. }, ValueKind::Cast { ty: b, .. })
            | (ValueKind::InstanceOf { ty: a, .. }, ValueKind::InstanceOf { ty: b, .. }) => a == b,
            (
                ValueKind::Field {
                    field: a,
                    receiver: x,
                },
                ValueKind::Field {
                    field: b,
                    receiver: y,
                },
            ) => a == b && x.is_some() == y.is_some(),
            (
                ValueKind::Update {
                    target: a,
                    increment: x,
                    postfix: p,
                },
                ValueKind::Update {
                    target: b,
                    increment: y,
                    postfix: q,
                },
            ) => {
                x == y
                    && p == q
                    && match (a, b) {
                        (
                            UpdateTarget::Field {
                                field: a,
                                receiver: x,
                            },
                            UpdateTarget::Field {
                                field: b,
                                receiver: y,
                            },
                        ) => a == b && x.is_some() == y.is_some(),
                        (
                            UpdateTarget::Array {
                                element_type: a, ..
                            },
                            UpdateTarget::Array {
                                element_type: b, ..
                            },
                        ) => a == b,
                        _ => false,
                    }
            }
            (
                ValueKind::CompoundUpdate {
                    target: a,
                    op: x,
                    operation_type: p,
                    ..
                },
                ValueKind::CompoundUpdate {
                    target: b,
                    op: y,
                    operation_type: q,
                    ..
                },
            ) => {
                x == y
                    && p == q
                    && match (a, b) {
                        (
                            UpdateTarget::Field {
                                field: a,
                                receiver: x,
                            },
                            UpdateTarget::Field {
                                field: b,
                                receiver: y,
                            },
                        ) => a == b && x.is_some() == y.is_some(),
                        (
                            UpdateTarget::Array {
                                element_type: a, ..
                            },
                            UpdateTarget::Array {
                                element_type: b, ..
                            },
                        ) => a == b,
                        _ => false,
                    }
            }
            (
                ValueKind::ArrayLoad {
                    element_type: a, ..
                },
                ValueKind::ArrayLoad {
                    element_type: b, ..
                },
            ) => a == b,
            (ValueKind::ArrayLength { .. }, ValueKind::ArrayLength { .. }) => true,
            (
                ValueKind::Call {
                    kind: a,
                    method: x,
                    receiver: r,
                    ..
                },
                ValueKind::Call {
                    kind: b,
                    method: y,
                    receiver: s,
                    ..
                },
            ) => a == b && x == y && r.is_some() == s.is_some(),
            (
                ValueKind::New {
                    class: a,
                    descriptor: x,
                    ..
                },
                ValueKind::New {
                    class: b,
                    descriptor: y,
                    ..
                },
            ) => a == b && x == y,
            (
                ValueKind::NewArray {
                    element_type: a, ..
                },
                ValueKind::NewArray {
                    element_type: b, ..
                },
            )
            | (
                ValueKind::ArrayInitializer {
                    element_type: a, ..
                },
                ValueKind::ArrayInitializer {
                    element_type: b, ..
                },
            ) => a == b,
            _ => false,
        };
        let definition = a.local_write().is_some() && b.local_write().is_some();
        let a = a.operands();
        let b = b.operands();
        attributes
            && a.len() == b.len()
            && a.into_iter()
                .zip(b)
                .all(|(a, b)| self.value(a, b, definition))
    }

    fn statement(&mut self, a: &Statement, b: &Statement) -> bool {
        let attributes = match (a, b) {
            (
                Statement::Assign { local: a, value: x },
                Statement::Assign { local: b, value: y },
            ) => {
                return self.local(*a, *b)
                    && std::mem::discriminant(x) == std::mem::discriminant(y)
                    && self.value(
                        x.value(),
                        y.value(),
                        matches!(x, crate::ir::ValueUse::Definition(_)),
                    );
            }
            (Statement::Eval(a), Statement::Eval(b)) => return self.value(*a, *b, true),
            (Statement::Monitor { enter: a, .. }, Statement::Monitor { enter: b, .. }) => a == b,
            (
                Statement::PutField {
                    field: a,
                    receiver: x,
                    ..
                },
                Statement::PutField {
                    field: b,
                    receiver: y,
                    ..
                },
            ) => a == b && x.is_some() == y.is_some(),
            (
                Statement::ArrayStore {
                    element_type: a, ..
                },
                Statement::ArrayStore {
                    element_type: b, ..
                },
            ) => a == b,
            (
                Statement::ConstructorCall { method: a, .. },
                Statement::ConstructorCall { method: b, .. },
            ) => a == b,
            _ => false,
        };
        let a = a.operands();
        let b = b.operands();
        attributes
            && a.len() == b.len()
            && a.into_iter().zip(b).all(|(a, b)| self.value(a, b, false))
    }

    fn predicate(&mut self, a: &Predicate, b: &Predicate) -> bool {
        a.equivalent_by(b, &mut |a, a_negated, b, b_negated| {
            self.point = Some((b, self.ir.blocks[b].statements.len()));
            let Terminator::Branch { condition: a, .. } = &self.ir.blocks[a].terminator else {
                return false;
            };
            let Terminator::Branch { condition: b, .. } = &self.ir.blocks[b].terminator else {
                return false;
            };
            let a_op = if a_negated { a.op.inverted() } else { a.op };
            let b_op = if b_negated { b.op.inverted() } else { b.op };
            a_op == b_op && self.value(a.left, b.left, false) && self.value(a.right, b.right, false)
        })
    }

    fn body(&mut self, a: &[Stmt], b: &[Stmt]) -> bool {
        // Cleanup proofs run before source normalization. Empty scheduled
        // fragments carry no operation and may occur at different transfer or
        // handler boundaries in otherwise identical cleanup copies.
        let substantive = |statement: &&Stmt| {
            !statement
                .scheduled(self.ir)
                .is_some_and(|scheduled| scheduled.statements.is_empty())
        };
        let mut a = a.iter().filter(substantive);
        let mut b = b.iter().filter(substantive);
        loop {
            match (a.next(), b.next()) {
                (None, None) => return true,
                (Some(a), Some(b)) if self.tree(a, b) => (),
                _ => return false,
            }
        }
    }

    fn tree(&mut self, a: &Stmt, b: &Stmt) -> bool {
        let enclosing = self.scope;
        if let Some(binders) = a.binder().zip(b.binder()) {
            self.scope = self.bindings.bind(enclosing, binders);
        }
        let valid = self.tree_inner(a, b);
        self.scope = enclosing;
        valid
    }

    /// Both references must resolve to the same paired lexical owner. Matching
    /// only a left-to-right map would miss shadowing on the right-hand side.
    fn same_binding<K: PartialEq>(
        &self,
        left: K,
        right: K,
        key: impl Fn(Binder) -> Option<K>,
    ) -> bool {
        let left = self
            .bindings
            .iter(self.scope)
            .position(|&(binder, _)| key(binder).is_some_and(|id| id == left));
        let right = self
            .bindings
            .iter(self.scope)
            .position(|&(_, binder)| key(binder).is_some_and(|id| id == right));
        left.is_some() && left == right
    }

    fn tree_inner(&mut self, a: &Stmt, b: &Stmt) -> bool {
        if let (Some(a), Some(b)) = (a.scheduled(self.ir), b.scheduled(self.ir)) {
            return a.statements.len() == b.statements.len()
                && a.iter().zip(b.iter()).all(|((_, left), (index, right))| {
                    self.point = Some((b.block, index));
                    self.statement(left, right)
                });
        }
        match (a, b) {
            (Stmt::Basic(a), Stmt::Basic(b)) => {
                let a = &self.ir.blocks[*a].statements;
                let b = &self.ir.blocks[*b].statements;
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| self.statement(a, b))
            }
            (Stmt::Throw(a), Stmt::Throw(b)) => {
                self.point = Some((*b, self.ir.blocks[*b].statements.len()));
                match (
                    &self.ir.blocks[*a].terminator,
                    &self.ir.blocks[*b].terminator,
                ) {
                    (Terminator::Throw { value: a, .. }, Terminator::Throw { value: b, .. }) => {
                        self.value(*a, *b, false)
                    }
                    _ => false,
                }
            }
            (
                Stmt::If {
                    condition: a,
                    then_body: x,
                    else_body: p,
                },
                Stmt::If {
                    condition: b,
                    then_body: y,
                    else_body: q,
                },
            ) => {
                let mut direct = self.clone();
                if direct.predicate(a, b) && direct.body(x, y) && direct.body(p, q) {
                    *self = direct;
                    return true;
                }
                self.predicate(a, &b.clone().negated()) && self.body(x, q) && self.body(p, y)
            }
            (Stmt::Label { body: x, .. }, Stmt::Label { body: y, .. }) => self.body(x, y),
            (
                Stmt::Loop {
                    body: x,
                    continuation: ac,
                    ..
                },
                Stmt::Loop {
                    body: y,
                    continuation: bc,
                    ..
                },
            ) => {
                self.body(x, y)
                    && match (ac, bc) {
                        (Some(a), Some(b)) => self.tree(&Stmt::Basic(*a), &Stmt::Basic(*b)),
                        (None, None) => true,
                        _ => false,
                    }
            }
            (
                Stmt::Switch {
                    head: ah, cases: x, ..
                },
                Stmt::Switch {
                    head: bh, cases: y, ..
                },
            ) => {
                let (
                    Terminator::Switch { value: a_value, .. },
                    Terminator::Switch { value: b_value, .. },
                ) = (
                    &self.ir.blocks[*ah].terminator,
                    &self.ir.blocks[*bh].terminator,
                )
                else {
                    return false;
                };
                self.point = Some((*bh, self.ir.blocks[*bh].statements.len()));
                self.value(*a_value, *b_value, false)
                    && x.len() == y.len()
                    && x.iter().zip(y).all(|(a, b)| {
                        a.keys == b.keys && a.default == b.default && self.body(&a.body, &b.body)
                    })
            }
            (Stmt::Break { target: a }, Stmt::Break { target: b }) => {
                self.same_binding(*a, *b, |binder| Some(binder.region()))
            }
            (Stmt::Continue { header: a }, Stmt::Continue { header: b }) => {
                self.same_binding(*a, *b, |binder| match binder {
                    Binder::Loop { header, .. } => Some(header),
                    _ => None,
                })
            }
            (
                Stmt::Synchronized {
                    monitor: a,
                    body: x,
                },
                Stmt::Synchronized {
                    monitor: b,
                    body: y,
                },
            ) => self.value(*a, *b, false) && self.body(x, y),
            (
                Stmt::TryFinally {
                    body: a,
                    cleanup: x,
                    ..
                },
                Stmt::TryFinally {
                    body: b,
                    cleanup: y,
                    ..
                },
            ) => self.body(a, b) && self.body(x, y),
            (
                Stmt::Try {
                    body: a,
                    catches: x,
                },
                Stmt::Try {
                    body: b,
                    catches: y,
                },
            ) => {
                self.body(a, b)
                    && x.len() == y.len()
                    && x.iter().zip(y).all(|(a, b)| {
                        let bindings = self.ir.blocks[a.handler]
                            .caught_exception
                            .and_then(|value| self.ir.values[value].local)
                            .zip(
                                self.ir.blocks[b.handler]
                                    .caught_exception
                                    .and_then(|value| self.ir.values[value].local),
                            );
                        a.catch_type == b.catch_type
                            && bindings.is_none_or(|(a, b)| self.local(a, b))
                            && self.body(&a.body, &b.body)
                    })
            }
            _ => false,
        }
    }
}

fn inline_write(ir: &MethodIr, value: ValueId, root: bool) -> bool {
    if !root && ir.values[value].local.is_some() {
        return false;
    }
    ir.values[value].kind.local_write().is_some()
        || ir.values[value]
            .kind
            .operands()
            .into_iter()
            .any(|value| inline_write(ir, value, false))
}

fn literal(a: &Literal, b: &Literal) -> bool {
    match (a, b) {
        (Literal::Null, Literal::Null) => true,
        (Literal::Int(a), Literal::Int(b)) => a == b,
        (Literal::Long(a), Literal::Long(b)) => a == b,
        (Literal::Float(a), Literal::Float(b)) => a.to_bits() == b.to_bits(),
        (Literal::Double(a), Literal::Double(b)) => a.to_bits() == b.to_bits(),
        (Literal::String(a), Literal::String(b)) => a == b,
        (Literal::Class(a), Literal::Class(b)) => a == b,
        _ => false,
    }
}

fn sites(ir: &MethodIr, body: &[Stmt]) -> BTreeSet<(BlockId, usize)> {
    let mut result = BTreeSet::new();
    let mut monitors = BTreeSet::new();
    for statement in tree::walk(body) {
        statement.operations(ir, &mut |operation| result.extend(operation.site()));
        if let Stmt::Synchronized { monitor, .. } = statement {
            monitors.insert(*monitor);
        }
    }
    // A proved synchronized region owns its same-value releases, including
    // exceptional release/rethrow operations omitted from the source body.
    // Those implicit uses must not make its temporary look live outside the
    // copied cleanup. Acquisitions remain explicit sites, so a use by another
    // scope still prevents alpha-renaming a shared local.
    if !monitors.is_empty() {
        for (block, data) in ir.blocks.iter_enumerated() {
            result.extend(data.statements.iter().enumerate().filter_map(|(index, statement)| {
                matches!(statement, Statement::Monitor { value, enter: false } if monitors.contains(value))
                    .then_some((block, index))
            }));
        }
    }
    result
}

fn reads_local(ir: &MethodIr, mut work: Vec<ValueId>, local: LocalId) -> bool {
    let mut seen = BTreeSet::new();
    while let Some(value) = work.pop() {
        if !seen.insert(value) {
            continue;
        }
        if ir.values[value].local == Some(local) {
            return true;
        }
        if ir.values[value].local.is_none() {
            work.extend(ir.values[value].kind.operands());
        }
    }
    false
}

fn internal(ir: &MethodIr, local: LocalId, region: &BTreeSet<(BlockId, usize)>) -> bool {
    ir.locals[local].parameter.is_none() && !ir.locals[local].is_this
        && ir.blocks.iter_enumerated().all(|(block,data)| {
            data.statements.iter().enumerate().all(|(index,statement)| region.contains(&(block,index))
                || !matches!(statement, Statement::Assign { local: found, .. } if *found == local) && !reads_local(ir, statement.operands(), local))
                && (region.contains(&(block,usize::MAX)) || !reads_local(ir,data.terminator.operands(),local))
        })
}

pub(super) fn reads_value(ir: &MethodIr, body: &[Stmt], target: ValueId) -> bool {
    let target_local = ir.values[target].local;
    let mut work = Vec::new();
    for statement in tree::walk(body) {
        statement.operations(ir, &mut |operation| work.extend(operation.operands()));
    }
    let mut seen = BTreeSet::new();
    while let Some(value) = work.pop() {
        if !seen.insert(value) {
            continue;
        }
        if value == target || target_local.is_some() && ir.values[value].local == target_local {
            return true;
        }
        work.extend(ir.values[value].kind.operands());
    }
    false
}

pub(super) fn written_locals(ir: &MethodIr, cleanup: &[Stmt]) -> BTreeSet<LocalId> {
    let mut writes = BTreeSet::new();
    for statement in tree::walk(cleanup) {
        statement.operations(ir, &mut |operation| {
            operation.local_effect(ir).writes(&mut |local| {
                writes.insert(local);
            });
        });
    }
    writes
}

pub(super) fn stable_return(ir: &MethodIr, value: ValueId, cleanup: &[Stmt]) -> bool {
    let writes = written_locals(ir, cleanup);
    let mut work = vec![value];
    let mut seen = BTreeSet::new();
    while let Some(value) = work.pop() {
        if !seen.insert(value) {
            continue;
        }
        if let Some(local) = ir.values[value].local {
            if writes.contains(&local) {
                return false;
            }
            continue;
        }
        match &ir.values[value].kind {
            ValueKind::Literal(_) | ValueKind::Parameter { .. } => (),
            ValueKind::Alias(_) | ValueKind::Unary { .. } | ValueKind::Compare { .. } => {
                work.extend(ir.values[value].kind.operands())
            }
            ValueKind::Binary { op, .. }
                if !matches!(op, crate::ir::BinaryOp::Div | crate::ir::BinaryOp::Rem) =>
            {
                work.extend(ir.values[value].kind.operands())
            }
            _ => return false,
        }
    }
    true
}
