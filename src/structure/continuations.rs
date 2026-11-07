//! Source continuations bind the result stores and terminal transfer of a
//! forward exit together. Result stores stay in their original cleanup and
//! monitor context. Only pure terminal transfers may cross those boundaries;
//! return operands additionally require preserved local reads.
pub(super) use super::bindings::Scope;
use super::bindings::Scopes;
use super::{Stmt, tree};
use crate::index::{BlockId, LocalId};
use crate::ir::scheduled::ValueUse;
use crate::ir::{MethodIr, Statement, Terminator, ValueKind, local_effects};
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
pub(super) enum ImmediateExit {
    Return(BlockId),
    Break(usize),
    Continue(BlockId),
}

impl ImmediateExit {
    fn statement(self) -> Stmt {
        match self {
            Self::Return(block) => Stmt::Return(block),
            Self::Break(target) => Stmt::Break { target },
            Self::Continue(header) => Stmt::Continue { header },
        }
    }

    #[cfg(test)]
    pub(super) fn return_expression(ir: &MethodIr, block: BlockId, constant: bool) -> bool {
        let Terminator::Return(value) = ir.blocks[block].terminator else {
            return false;
        };
        value.is_none_or(|value| pure_expression(ir, ValueUse::Reference(value), constant))
    }
}

/// These small source expressions commute with control and exception dispatch.
/// The budget bounds expanded source, so repeated operands count separately.
fn pure_expression(ir: &MethodIr, usage: ValueUse, constant: bool) -> bool {
    let mut cost = 0;
    let mut pending = vec![usage];
    while let Some(usage) = pending.pop() {
        cost += 1;
        if cost > 12 {
            return false;
        }
        if usage.local(ir).is_some() {
            if constant {
                return false;
            }
            continue;
        }
        let value = &ir.values[usage.value()];
        if let ValueKind::Literal(crate::ir::Literal::String(text)) = &value.kind {
            cost += text.len().div_ceil(16);
            if cost > 12 {
                return false;
            }
        }
        if value.may_throw()
            || !matches!(
                value.kind,
                ValueKind::Literal(_)
                    | ValueKind::Alias(_)
                    | ValueKind::Unary { .. }
                    | ValueKind::Binary { .. }
                    | ValueKind::Compare { .. }
                    | ValueKind::Cast { .. }
                    | ValueKind::Select { .. }
            )
        {
            return false;
        }
        usage.for_each_child(ir, |child| pending.push(child));
    }
    true
}

#[derive(Clone)]
pub(super) struct Continuation {
    stores: Vec<Stmt>,
    exit: ImmediateExit,
    operations: usize,
}

impl From<ImmediateExit> for Continuation {
    fn from(exit: ImmediateExit) -> Self {
        Self {
            stores: Vec::new(),
            exit,
            operations: 0,
        }
    }
}

impl Continuation {
    /// New loop/switch exits may expose direct results, which source lowering
    /// can join with their owning stores. Keep a shared calculation or store
    /// prefix at its existing continuation instead of repeating it per arm.
    pub(super) fn direct_return(&self, ir: &MethodIr) -> bool {
        let ImmediateExit::Return(block) = self.exit else {
            return false;
        };
        if !self.stores.is_empty() {
            return false;
        }
        let Terminator::Return(value) = ir.blocks[block].terminator else {
            return false;
        };
        let Some(mut value) = value else {
            return true;
        };
        for _ in 0..ir.values.len() {
            if ValueUse::Reference(value).local(ir).is_some() {
                return true;
            }
            match ir.values[value].kind {
                ValueKind::Literal(_) => return true,
                ValueKind::Alias(inner) => value = inner,
                _ => return false,
            }
        }
        false
    }

    /// Compute the continuation before a normalized source fragment. A value
    /// prefix needs a known terminal destination: copying an arbitrary suffix
    /// would move throwing code and lose lexical handler ownership.
    pub(super) fn before(ir: &MethodIr, statement: &Stmt, next: Option<Self>) -> Option<Self> {
        let exit = match statement {
            Stmt::Return(block) => {
                let Terminator::Return(value) = ir.blocks[*block].terminator else {
                    return None;
                };
                if value
                    .is_some_and(|value| !pure_expression(ir, ValueUse::Reference(value), false))
                {
                    return None;
                }
                Some(ImmediateExit::Return(*block))
            }
            Stmt::Break { target } => Some(ImmediateExit::Break(*target)),
            Stmt::Continue { header } => Some(ImmediateExit::Continue(*header)),
            _ => None,
        };
        if let Some(exit) = exit {
            return Some(exit.into());
        }
        let scheduled = statement.scheduled(ir)?;
        let mut next = next?;
        if scheduled.statements.is_empty() {
            return Some(next);
        }
        // The loop owns its iteration completion, including updates shared by
        // guard/continue paths. Copying that update into individual exits hides
        // the counted loop boundary. Value exits use break or return instead.
        if matches!(next.exit, ImmediateExit::Continue(_)) {
            return None;
        }
        next.operations += scheduled.statements.len();
        if next.operations > 4 {
            return None;
        }
        for statement in scheduled.statements {
            let Statement::Assign { value, .. } = *statement else {
                return None;
            };
            if !pure_expression(ir, value, false) {
                return None;
            }
        }
        next.stores.insert(0, statement.clone());
        Some(next)
    }

    fn statements(&self) -> impl Iterator<Item = Stmt> + '_ {
        self.stores
            .iter()
            .cloned()
            .chain(std::iter::once(self.exit.statement()))
    }

    fn crosses(&self, ir: &MethodIr, cleanup: &CleanupWrites) -> bool {
        // Cleanup may throw or override the pending exit. In either case a
        // result store originally after it might never execute. Keep stores
        // in their lexical cleanup/monitor context rather than reconstructing
        // all possible observers after exceptional or abrupt resumption.
        if !self.stores.is_empty() {
            return false;
        }
        let ImmediateExit::Return(block) = self.exit else {
            return true;
        };
        let mut reads = BTreeSet::new();
        local_effects::terminator(ir, &ir.blocks[block].terminator, false)
            .reads_before_writes(&mut reads, &mut BTreeSet::new());
        reads.is_empty() || (!cleanup.opaque && reads.is_disjoint(&cleanup.writes))
    }
}

/// The existing ordered scheduled effects decide which return operands can
/// be evaluated before cleanup. Result stores use a lexical boundary instead.
#[derive(Default)]
struct CleanupWrites {
    writes: BTreeSet<LocalId>,
    opaque: bool,
}

impl CleanupWrites {
    fn of(ir: &MethodIr, body: &[Stmt]) -> Self {
        let mut result = Self::default();
        for statement in tree::walk(body) {
            statement.operations(ir, &mut |operation| {
                operation.local_effect(ir).writes(&mut |local| {
                    result.writes.insert(local);
                });
            });
            if matches!(statement, Stmt::Dispatch { .. } | Stmt::BoundRegion { .. }) {
                result.opaque = true;
            }
        }
        result
    }
}

/// Persistent lexical environments are arena indices, not recursively owned
/// lists. Deep forward ladders therefore need neither subtree rewrites nor a
/// recursive drop chain. Each transfer consults the binders it actually exits.
#[derive(Default)]
pub(super) struct Bindings(Scopes<Boundary>);
enum Boundary {
    Region {
        region: usize,
        continuation: Option<Continuation>,
    },
    Cleanup(CleanupWrites),
}

impl Bindings {
    fn push(&mut self, parent: Scope, boundary: Boundary) -> Scope {
        self.0.bind(parent, boundary)
    }

    pub(super) fn bind(
        &mut self,
        parent: Scope,
        region: usize,
        continuation: Option<Continuation>,
    ) -> Scope {
        self.push(
            parent,
            Boundary::Region {
                region,
                continuation,
            },
        )
    }

    pub(super) fn children(&mut self, ir: &MethodIr, statement: &Stmt, scope: Scope) -> Vec<Scope> {
        match statement {
            Stmt::TryFinally { cleanup, .. } => {
                let protected = self.push(scope, Boundary::Cleanup(CleanupWrites::of(ir, cleanup)));
                vec![protected, scope]
            }
            Stmt::Synchronized { .. } => {
                vec![self.push(scope, Boundary::Cleanup(CleanupWrites::default()))]
            }
            _ => vec![scope; statement.bodies().count()],
        }
    }

    pub(super) fn transfer(&self, ir: &MethodIr, target: usize, scope: Scope) -> Vec<Stmt> {
        let unchanged = || vec![Stmt::Break { target }];
        let mut barriers = Vec::new();
        let mut shadowed = Vec::new();
        for boundary in self.0.iter(scope) {
            match boundary {
                Boundary::Region {
                    region,
                    continuation,
                } if *region == target => {
                    let Some(continuation) = continuation else {
                        return unchanged();
                    };
                    if matches!(continuation.exit, ImmediateExit::Break(destination)
                        if destination == target || shadowed.contains(&destination))
                    {
                        return unchanged();
                    }
                    if barriers
                        .into_iter()
                        .any(|cleanup| !continuation.crosses(ir, cleanup))
                    {
                        return unchanged();
                    }
                    return continuation.statements().collect();
                }
                Boundary::Region { region, .. } => shadowed.push(*region),
                Boundary::Cleanup(cleanup) => barriers.push(cleanup),
            }
        }
        unchanged()
    }
}

#[cfg(test)]
pub(super) fn redirect_exits(
    ir: &MethodIr,
    body: &mut Vec<Stmt>,
    target: usize,
    continuation: &Continuation,
) {
    fn descend(ir: &MethodIr, body: Vec<Stmt>, bindings: &mut Bindings, scope: Scope) -> Vec<Stmt> {
        let mut result = Vec::new();
        for mut statement in body {
            if let Stmt::Break { target } = statement {
                result.extend(bindings.transfer(ir, target, scope));
                continue;
            }
            let nested = statement
                .binder()
                .map_or(scope, |binder| bindings.bind(scope, binder.region(), None));
            let scopes = bindings.children(ir, &statement, nested);
            for (body, scope) in statement.bodies_mut().zip(scopes) {
                *body = descend(ir, std::mem::take(body), bindings, scope);
            }
            result.push(statement);
        }
        result
    }
    let mut bindings = Bindings::default();
    let scope = bindings.bind(None, target, Some(continuation.clone()));
    *body = descend(ir, std::mem::take(body), &mut bindings, scope);
}
