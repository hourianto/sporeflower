//! Structural children and lexical region binders. Child bodies are visited in
//! source order; scheduled loop continuations are block references, not bodies.
use super::{BlockId, RegionId, Stmt};
use crate::index::ValueId;
use crate::ir::{MethodIr, Statement, Terminator, ValueUse, local_effects};

/// An operation owned directly by a structured node. Child bodies are separate;
/// this is an inventory for proofs, not an execution schedule across branches.
pub(crate) enum Operation<'a> {
    Statement {
        block: BlockId,
        index: usize,
        statement: &'a Statement,
    },
    Terminator {
        block: BlockId,
        terminator: &'a Terminator,
    },
    Expression(ValueId),
    CatchBinding(ValueId),
}

impl Operation<'_> {
    pub(crate) fn site(&self) -> Option<(BlockId, usize)> {
        match *self {
            Self::Statement { block, index, .. } => Some((block, index)),
            Self::Terminator { block, .. } => Some((block, usize::MAX)),
            Self::Expression(_) | Self::CatchBinding(_) => None,
        }
    }

    pub(crate) fn operands(&self) -> Vec<ValueId> {
        match self {
            Self::Statement { statement, .. } => statement.operands(),
            Self::Terminator { terminator, .. } => terminator.operands(),
            Self::Expression(value) => vec![*value],
            Self::CatchBinding(_) => Vec::new(),
        }
    }

    pub(crate) fn local_effect(&self, ir: &MethodIr) -> local_effects::Effect {
        match *self {
            Self::Statement { statement, .. } => local_effects::statement(ir, statement, false),
            Self::Terminator { terminator, .. } => local_effects::terminator(ir, terminator, false),
            Self::Expression(value) => local_effects::expression(ir, ValueUse::Reference(value)),
            Self::CatchBinding(value) => {
                ir.values[value]
                    .local
                    .map_or(local_effects::Effect::Empty, |local| {
                        local_effects::Effect::Write {
                            local,
                            copied: None,
                            value: ValueUse::Definition(value),
                        }
                    })
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Binder {
    Loop { region: RegionId, header: BlockId },
    Label(RegionId),
    Switch(RegionId),
    Dispatch(RegionId),
}

impl Binder {
    pub(crate) fn region(self) -> RegionId {
        match self {
            Self::Loop { region, .. }
            | Self::Label(region)
            | Self::Switch(region)
            | Self::Dispatch(region) => region,
        }
    }
}

impl Stmt {
    /// Includes exact statement fragments, predicate operands, catch bindings
    /// and owned loop updates. Opaque-region policies remain with each proof.
    pub(crate) fn operations<'a>(&self, ir: &'a MethodIr, visit: &mut impl FnMut(Operation<'a>)) {
        let scheduled = match self {
            Self::Loop {
                continuation: Some(block),
                ..
            } => Self::Basic(*block).scheduled(ir),
            _ => self.scheduled(ir),
        };
        if let Some(scheduled) = scheduled {
            for (offset, statement) in scheduled.statements.iter().enumerate() {
                visit(Operation::Statement {
                    block: scheduled.block,
                    index: scheduled.start + offset,
                    statement,
                });
            }
        }
        let mut terminator = |block: BlockId| {
            visit(Operation::Terminator {
                block,
                terminator: &ir.blocks[block].terminator,
            });
        };
        match self {
            Self::If { condition, .. } => {
                for block in condition.heads() {
                    terminator(block);
                }
            }
            Self::Switch { head, .. } | Self::Throw(head) | Self::Return(head) => terminator(*head),
            Self::Synchronized { monitor, .. } => visit(Operation::Expression(*monitor)),
            Self::Try { catches, .. } => {
                for catch in catches {
                    if let Some(value) = ir.blocks[catch.handler].caught_exception {
                        visit(Operation::CatchBinding(value));
                    }
                }
            }
            _ => (),
        }
    }

    /// Lexical bodies, including opaque-region exit bindings. A dispatcher's
    /// outgoing region IDs are references in the enclosing scope, not bodies.
    pub(crate) fn bodies(&self) -> impl DoubleEndedIterator<Item = &Vec<Stmt>> {
        let (first, second, cases, catches, dispatch, exits) = match self {
            Self::If {
                then_body,
                else_body,
                ..
            } => (Some(then_body), Some(else_body), None, None, None, None),
            Self::TryFinally { body, cleanup, .. } => {
                (Some(body), Some(cleanup), None, None, None, None)
            }
            Self::Loop { body, .. }
            | Self::Label { body, .. }
            | Self::Synchronized { body, .. } => (Some(body), None, None, None, None, None),
            Self::Try { body, catches } => (Some(body), None, None, Some(catches), None, None),
            Self::Switch { cases, .. } => (None, None, Some(cases), None, None, None),
            Self::Dispatch { cases, .. } => (None, None, None, None, Some(cases), None),
            Self::BoundRegion { exits, .. } => (None, None, None, None, None, Some(exits)),
            Self::Basic(_)
            | Self::Statements { .. }
            | Self::Return(_)
            | Self::Throw(_)
            | Self::Unreachable(_)
            | Self::Break { .. }
            | Self::Continue { .. }
            | Self::DispatchContinue { .. }
            | Self::Jump(_) => (None, None, None, None, None, None),
        };
        first
            .into_iter()
            .chain(second)
            .chain(cases.into_iter().flatten().map(|case| &case.body))
            .chain(catches.into_iter().flatten().map(|catch| &catch.body))
            .chain(dispatch.into_iter().flatten().map(|case| &case.body))
            .chain(exits.into_iter().flatten().map(|(_, body)| body))
    }

    pub(crate) fn bodies_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut Vec<Stmt>> {
        let (first, second, cases, catches, dispatch, exits) = match self {
            Self::If {
                then_body,
                else_body,
                ..
            } => (Some(then_body), Some(else_body), None, None, None, None),
            Self::TryFinally { body, cleanup, .. } => {
                (Some(body), Some(cleanup), None, None, None, None)
            }
            Self::Loop { body, .. }
            | Self::Label { body, .. }
            | Self::Synchronized { body, .. } => (Some(body), None, None, None, None, None),
            Self::Try { body, catches } => (Some(body), None, None, Some(catches), None, None),
            Self::Switch { cases, .. } => (None, None, Some(cases), None, None, None),
            Self::Dispatch { cases, .. } => (None, None, None, None, Some(cases), None),
            Self::BoundRegion { exits, .. } => (None, None, None, None, None, Some(exits)),
            Self::Basic(_)
            | Self::Statements { .. }
            | Self::Return(_)
            | Self::Throw(_)
            | Self::Unreachable(_)
            | Self::Break { .. }
            | Self::Continue { .. }
            | Self::DispatchContinue { .. }
            | Self::Jump(_) => (None, None, None, None, None, None),
        };
        first
            .into_iter()
            .chain(second)
            .chain(cases.into_iter().flatten().map(|case| &mut case.body))
            .chain(catches.into_iter().flatten().map(|catch| &mut catch.body))
            .chain(dispatch.into_iter().flatten().map(|case| &mut case.body))
            .chain(exits.into_iter().flatten().map(|(_, body)| body))
    }

    pub(crate) fn map_bodies(mut self, mut map: impl FnMut(Vec<Stmt>) -> Vec<Stmt>) -> Self {
        for body in self.bodies_mut() {
            *body = map(std::mem::take(body));
        }
        self
    }

    pub(crate) fn binder(&self) -> Option<Binder> {
        match *self {
            Self::Loop { region, header, .. } => Some(Binder::Loop { region, header }),
            Self::Label { region, .. } => Some(Binder::Label(region)),
            Self::Switch { region, .. } => Some(Binder::Switch(region)),
            Self::Dispatch { region, .. } => Some(Binder::Dispatch(region)),
            _ => None,
        }
    }

    pub(super) fn region_mut(&mut self) -> Option<&mut RegionId> {
        match self {
            Self::Loop { region, .. }
            | Self::Label { region, .. }
            | Self::Switch { region, .. }
            | Self::Dispatch { region, .. } => Some(region),
            _ => None,
        }
    }
}

/// Preorder traversal of every actual statement, including temporary regions.
pub(crate) fn walk(body: &[Stmt]) -> impl Iterator<Item = &Stmt> {
    let mut pending: Vec<_> = body.iter().rev().collect();
    std::iter::from_fn(move || {
        let statement = pending.pop()?;
        for body in statement.bodies().rev() {
            pending.extend(body.iter().rev());
        }
        Some(statement)
    })
}

/// The callback sees enclosing binders only. Repeated region IDs and loop
/// headers are resolved from the end, so sibling or nested recovery may reuse
/// identities until renumbering. BoundRegion adds no lexical binder.
pub(crate) fn walk_scoped(body: &[Stmt], visit: &mut impl FnMut(&Stmt, &[Binder])) {
    fn descend(body: &[Stmt], active: &mut Vec<Binder>, visit: &mut impl FnMut(&Stmt, &[Binder])) {
        for statement in body {
            visit(statement, active);
            let binder = statement.binder();
            active.extend(binder);
            for body in statement.bodies() {
                descend(body, active, visit);
            }
            if binder.is_some() {
                active.pop();
            }
        }
    }
    descend(body, &mut Vec::new(), visit);
}

#[cfg(test)]
#[path = "../../tests/unit/structure_tree.rs"]
mod tests;
