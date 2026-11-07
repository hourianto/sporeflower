//! Lexical environments and multi-exit region bindings. Recovery-local IDs may
//! be reused: leaving a child scope must reveal its enclosing binding again.
use super::{BlockId, RegionId, Stmt};

/// Persistent scope handles allow sibling traversals and speculative matchers
/// to share enclosing bindings without recursive ownership or map restoration.
pub(super) type Scope = Option<usize>;

#[derive(Clone)]
pub(super) struct Scopes<T>(Vec<(Scope, T)>);

impl<T> Default for Scopes<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T> Scopes<T> {
    pub(super) fn bind(&mut self, parent: Scope, binding: T) -> Scope {
        let index = self.0.len();
        self.0.push((parent, binding));
        Some(index)
    }

    /// Innermost binding first. The caller selects the relevant namespace or
    /// boundary; unrelated loop, region and cleanup bindings stay distinct.
    pub(super) fn iter(&self, mut scope: Scope) -> impl Iterator<Item = &T> {
        std::iter::from_fn(move || {
            let (parent, binding) = &self.0[scope?];
            scope = *parent;
            Some(binding)
        })
    }
}

pub(super) fn renumber_regions_from(body: &mut [Stmt], start: RegionId) -> RegionId {
    fn remap(target: &mut RegionId, scopes: &Scopes<(RegionId, RegionId)>, scope: Scope) {
        if let Some(&(_, new)) = scopes.iter(scope).find(|&&(old, _)| old == *target) {
            *target = new;
        }
    }
    fn visit(
        body: &mut [Stmt],
        scopes: &mut Scopes<(RegionId, RegionId)>,
        scope: Scope,
        next: &mut RegionId,
    ) {
        for statement in body {
            let nested = statement.region_mut().map_or(scope, |region| {
                let nested = scopes.bind(scope, (*region, *next));
                *region = *next;
                *next += 1;
                nested
            });
            match statement {
                Stmt::Break { target } | Stmt::DispatchContinue { region: target, .. } => {
                    remap(target, scopes, scope);
                }
                _ => (),
            }
            for body in statement.bodies_mut() {
                visit(body, scopes, nested, next);
            }
            // Dispatcher exits belong to its enclosing scope even when the
            // dispatcher itself reuses an enclosing region ID.
            if let Stmt::Dispatch { exits, .. } = statement {
                for (_, target) in exits {
                    remap(target, scopes, scope);
                }
            }
        }
    }
    let mut next = start;
    visit(body, &mut Scopes::default(), None, &mut next);
    next
}

pub(super) fn bind(mut body: Vec<Stmt>, exits: &[(BlockId, Vec<Stmt>)]) -> Vec<Stmt> {
    let base = exits
        .iter()
        .flat_map(|(_, body)| body.iter())
        .filter_map(|statement| match statement {
            Stmt::Break { target } => Some(*target),
            Stmt::DispatchContinue { region, .. } => Some(*region),
            _ => None,
        })
        .max()
        .map_or(0, |id| id + 1);
    // Inner IDs must not shadow an enclosing exit label. A final traversal
    // allocates compact deterministic IDs after every region has been composed.
    super::renumber_regions_from(&mut body, base + 1);
    // A fallthrough binding still exits the entire opaque region. Removing a
    // terminal Jump in one arm would otherwise fall into another exit arm.
    let exits: Vec<_> = exits
        .iter()
        .map(|(target, transfer)| {
            (
                *target,
                if transfer.is_empty() {
                    vec![Stmt::Break { target: base }]
                } else {
                    transfer.clone()
                },
            )
        })
        .collect();
    let body = replace(body, &exits);
    if super::contains_break(&body, base) {
        vec![Stmt::Label { region: base, body }]
    } else {
        body
    }
}

fn replace(body: Vec<Stmt>, exits: &[(BlockId, Vec<Stmt>)]) -> Vec<Stmt> {
    let mut result = Vec::new();
    for statement in body {
        match statement {
            Stmt::Jump(target) => {
                if let Some((_, replacement)) = exits.iter().find(|(exit, _)| *exit == target) {
                    result.extend(replacement.clone());
                } else {
                    result.push(Stmt::Jump(target));
                }
            }
            other => result.push(other.map_bodies(|body| replace(body, exits))),
        }
    }
    result
}
