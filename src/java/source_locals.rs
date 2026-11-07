//! Bindings belong to the completed source scopes. Value choices and scheduled
//! loop ownership settle first; actual scopes then decide post-test spelling.
//! Scheduled identities supply accesses, never names parsed from rendered Java.
use super::source::{Body, Expression, LoopHeader, Selector, Simple, Statement, access};
use crate::{
    index::IdVec,
    ir::{LocalId, MethodIr, ValueUse},
};
use std::collections::{BTreeSet, HashSet};

pub(super) fn place(
    ir: &MethodIr,
    body: &mut Body,
    caught: &HashSet<LocalId>,
    selected: Option<&BTreeSet<LocalId>>,
    initialize: bool,
) {
    let mut scan = Scan {
        ir,
        uses: vec![LocalUse::default(); ir.locals.len()].into(),
        scopes: vec![Scope {
            parent: 0,
            end: 0,
            insertable: true,
        }],
        assignment: 0,
    };
    walk(body, 0, &mut scan);
    scan.scopes[0].end = scan.scopes.len();
    let live = ir
        .locals
        .iter()
        .any(|local| local.needs_default)
        .then(|| super::local_flow::live_at_entry(ir).ok())
        .flatten();
    let mut declarations = vec![Vec::new(); scan.scopes.len()];
    let mut assignments = BTreeSet::new();
    for (local, facts) in scan.uses.iter_enumerated() {
        let binding = &ir.locals[local];
        if !facts.seen
            || binding.is_this
            || binding.parameter.is_some()
            || caught.contains(&local)
            || selected.is_some_and(|selected| !selected.contains(&local))
        {
            continue;
        }
        if !initialize
            && let Some((scope, assignment)) = facts.first_assignment
            && facts.min_scope >= scope
            && facts.max_scope < scan.scopes[scope].end
            && (!binding.needs_default || live.as_ref().is_some_and(|live| !live.contains(&local)))
        {
            assignments.insert(assignment);
        } else {
            // An initialized entry value may carry state across iterations.
            // Narrowing its lexical scope must not repeat that initialization.
            let mut scope = if initialize
                || binding.needs_default && live.as_ref().is_none_or(|live| live.contains(&local))
            {
                0
            } else {
                facts.common_scope
            };
            while !scan.scopes[scope].insertable {
                scope = scan.scopes[scope].parent;
            }
            declarations[scope].push(local);
        }
    }
    let counts = scan.uses.iter().map(|facts| facts.accesses).collect();
    let mut apply = Apply {
        ir,
        assignments,
        declarations,
        counts,
        assignment: 0,
        scope: 1,
        initialize,
    };
    walk(body, 0, &mut apply);
}

#[derive(Clone, Default)]
struct LocalUse {
    seen: bool,
    first_assignment: Option<(usize, usize)>,
    min_scope: usize,
    max_scope: usize,
    common_scope: usize,
    accesses: usize,
}
struct Scope {
    parent: usize,
    end: usize,
    insertable: bool,
}
struct Scan<'a> {
    ir: &'a MethodIr,
    uses: IdVec<LocalId, LocalUse>,
    scopes: Vec<Scope>,
    assignment: usize,
}

impl Scan<'_> {
    fn local(&mut self, local: LocalId, scope: usize) {
        let facts = &mut self.uses[local];
        facts.accesses += 1;
        if !facts.seen {
            facts.seen = true;
            facts.min_scope = scope;
            facts.max_scope = scope;
            facts.common_scope = scope;
        } else {
            facts.min_scope = facts.min_scope.min(scope);
            facts.max_scope = facts.max_scope.max(scope);
            let mut left = facts.common_scope;
            let mut right = scope;
            while left != right {
                if left > right {
                    left = self.scopes[left].parent;
                } else {
                    right = self.scopes[right].parent;
                }
            }
            facts.common_scope = left;
        }
    }
}

trait Visitor {
    fn enter(&mut self, parent: usize, insertable: bool) -> usize;
    fn leave(&mut self, _scope: usize) {}
    fn simple(&mut self, _value: &mut Simple, _scope: usize) {}
    fn expression(&mut self, _value: &Expression, _scope: usize) {}
    fn locals(&mut self, _values: &BTreeSet<LocalId>, _scope: usize) {}
    fn selector(&mut self, _value: &Selector, _scope: usize) {}
    fn throwing(&mut self, _value: &super::source::Throw, _scope: usize) {}
    fn finish(&mut self, _body: &mut Body, _scope: usize) {}
}
impl Visitor for Scan<'_> {
    fn enter(&mut self, parent: usize, insertable: bool) -> usize {
        let scope = self.scopes.len();
        self.scopes.push(Scope {
            parent,
            end: 0,
            insertable,
        });
        scope
    }
    fn leave(&mut self, scope: usize) {
        self.scopes[scope].end = self.scopes.len();
    }
    fn simple(&mut self, value: &mut Simple, scope: usize) {
        if let Simple::Assign { local, value, .. } = value {
            self.expression(value, scope);
            if !self.uses[*local].seen {
                self.uses[*local].first_assignment = Some((scope, self.assignment));
            }
            self.local(*local, scope);
            self.assignment += 1;
        } else {
            self.locals(&access::simple(self.ir, value), scope);
        }
    }
    fn expression(&mut self, value: &Expression, scope: usize) {
        self.locals(&access::expression(self.ir, value), scope);
    }
    fn locals(&mut self, values: &BTreeSet<LocalId>, scope: usize) {
        for &local in values {
            self.local(local, scope);
        }
    }
    fn throwing(&mut self, value: &super::source::Throw, scope: usize) {
        self.locals(&access::throwing(self.ir, value), scope);
    }
    fn selector(&mut self, value: &Selector, scope: usize) {
        if let Selector::Value(value) = value {
            self.locals(
                &super::local_flow::value_accesses(self.ir, ValueUse::Reference(*value)),
                scope,
            );
        }
    }
}
struct Apply<'a> {
    ir: &'a MethodIr,
    assignments: BTreeSet<usize>,
    declarations: Vec<Vec<LocalId>>,
    counts: IdVec<LocalId, usize>,
    assignment: usize,
    scope: usize,
    initialize: bool,
}
impl Visitor for Apply<'_> {
    fn enter(&mut self, _parent: usize, _insertable: bool) -> usize {
        let scope = self.scope;
        self.scope += 1;
        scope
    }
    fn simple(&mut self, value: &mut Simple, _scope: usize) {
        if let Simple::Assign { declaration, .. } = value {
            *declaration = self.assignments.contains(&self.assignment);
            self.assignment += 1;
        }
    }
    fn finish(&mut self, body: &mut Body, scope: usize) {
        *body = std::mem::take(body)
            .into_iter()
            .flat_map(|statement| super::source::loops::finish(self.ir, statement, &self.counts))
            .collect();
        // A single saved monitor use consumes the already chosen declaration.
        // Its initializer and monitor remain in the same enclosing scope.
        let mut index = 1;
        while index < body.len() {
            let monitor = match &body[index] {
                Statement::Synchronized {
                    monitor: Expression::Value(ValueUse::Reference(value)),
                    ..
                } => self.ir.values[*value].local,
                _ => None,
            };
            if let Some(local) = monitor
                && self.counts[local] == 2
                && matches!(&body[index - 1], Statement::Simple(Simple::Assign { local: assigned, declaration: true, .. }) if *assigned == local)
            {
                let Statement::Simple(Simple::Assign { value, .. }) = body.remove(index - 1) else {
                    unreachable!()
                };
                let Statement::Synchronized { monitor, .. } = &mut body[index - 1] else {
                    unreachable!()
                };
                *monitor = value;
            } else {
                index += 1;
            }
        }
        let declarations = self.declarations[scope].iter().map(|&local| {
            Statement::Simple(Simple::Declare {
                local,
                initialize: self.initialize || self.ir.locals[local].needs_default,
            })
        });
        body.splice(0..0, declarations);
    }
}

fn nested(body: &mut Body, parent: usize, visitor: &mut impl Visitor) {
    let scope = visitor.enter(parent, true);
    walk(body, scope, visitor);
    visitor.leave(scope);
}
fn walk(body: &mut Body, scope: usize, visitor: &mut impl Visitor) {
    for statement in body.iter_mut() {
        match statement {
            Statement::Simple(value) => visitor.simple(value, scope),
            Statement::Return(Some(value)) => visitor.expression(value, scope),
            Statement::Throw(value) => visitor.throwing(value, scope),
            Statement::If { condition, yes, no } => {
                visitor.expression(condition, scope);
                nested(yes, scope, visitor);
                nested(no, scope, visitor);
            }
            Statement::Loop { header, body, .. } => match header {
                LoopHeader::For {
                    initializer,
                    condition,
                    update,
                } => {
                    let inner = visitor.enter(scope, false);
                    if let Some(initializer) = initializer {
                        visitor.simple(initializer, inner);
                    }
                    visitor.expression(condition, inner);
                    nested(body, inner, visitor);
                    visitor.simple(update, inner);
                    visitor.leave(inner);
                }
                LoopHeader::While(condition) => {
                    visitor.expression(condition, scope);
                    nested(body, scope, visitor);
                }
                LoopHeader::DoWhile(condition) => {
                    nested(body, scope, visitor);
                    visitor.expression(condition, scope);
                }
            },
            Statement::Switch { value, cases, .. } => {
                visitor.selector(value, scope);
                for case in cases {
                    nested(&mut case.body, scope, visitor);
                }
            }
            Statement::Synchronized { monitor, body } => {
                visitor.expression(monitor, scope);
                nested(body, scope, visitor);
            }
            Statement::Block { body, .. } => nested(body, scope, visitor),
            Statement::Try {
                body,
                catches,
                cleanup,
            } => {
                nested(body, scope, visitor);
                for catch in catches {
                    nested(&mut catch.body, scope, visitor);
                }
                if let Some(cleanup) = cleanup {
                    nested(cleanup, scope, visitor);
                }
            }
            _ => (),
        }
    }
    visitor.finish(body, scope);
}

#[cfg(test)]
#[path = "../../tests/unit/source_locals.rs"]
mod tests;
