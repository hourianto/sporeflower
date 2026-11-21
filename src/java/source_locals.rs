//! Final source scopes own declarations. Binding freezes the eligible locals
//! and required entry values before normalization; this solver reads only the
//! completed bound tree after choices, substitutions and loops have settled.
use super::source::{
    bound::{AssignmentValue, Binding, Body, LoopHeader, Simple, Statement},
    evaluation,
};
use crate::{index::IdVec, ir::LocalId, java::syntax::Expr};

pub(in crate::java) type Declarations = IdVec<LocalId, Option<Declaration>>;
pub(in crate::java) struct Declaration {
    pub binding: Binding,
    pub initialize: bool,
    /// Entry values must not be reinitialized on each loop iteration.
    pub entry: bool,
}

pub(super) fn place(body: &mut Body, inputs: Declarations) {
    // The bound invocation can read or update parameters. Java 1.3 starts the
    // scope for new constructor locals immediately after that first statement.
    let entry = usize::from(matches!(
        body.first(),
        Some(Statement::Simple(Simple::Delegation(_)))
    ));
    let mut scan = Scan {
        uses: vec![None; inputs.len()].into(),
        scopes: vec![Scope {
            parent: 0,
            parent_statement: 0,
            statement: 0,
            insertable: true,
        }],
    };
    walk(body, 0, &mut scan);
    let mut declarations: Vec<Vec<(usize, Declaration)>> =
        (0..scan.scopes.len()).map(|_| Vec::new()).collect();
    let mut assignments: IdVec<LocalId, bool> = vec![false; inputs.len()].into();
    for (local, declaration) in inputs.into_iter_enumerated() {
        let (Some(declaration), Some(facts)) = (declaration, &scan.uses[local]) else {
            continue;
        };
        if declaration.entry {
            declarations[0].push((entry, declaration));
            continue;
        }
        // An initial assignment can declare the local when its scope encloses
        // every occurrence, including reads in nested blocks.
        let (mut first_scope, mut statement) = facts.first_occurrence;
        let mut scope = facts.common_scope;
        if facts.starts_with_assignment && first_scope == scope {
            assignments[local] = true;
            continue;
        }
        while !scan.scopes[scope].insertable {
            scope = scan.scopes[scope].parent;
        }
        while first_scope != scope {
            statement = scan.scopes[first_scope].parent_statement;
            first_scope = scan.scopes[first_scope].parent;
        }
        declarations[scope].push((statement, declaration));
    }
    let mut apply = Apply {
        assignments,
        declarations,
        scope: 1,
    };
    walk(body, 0, &mut apply);
}

#[derive(Clone)]
struct LocalUse {
    starts_with_assignment: bool,
    first_occurrence: (usize, usize),
    common_scope: usize,
}
struct Scope {
    parent: usize,
    parent_statement: usize,
    statement: usize,
    insertable: bool,
}
struct Scan {
    uses: IdVec<LocalId, Option<LocalUse>>,
    scopes: Vec<Scope>,
}

impl Scan {
    fn local(&mut self, local: LocalId, scope: usize, assignment: bool) {
        let facts = self.uses[local].get_or_insert_with(|| LocalUse {
            starts_with_assignment: assignment,
            first_occurrence: (scope, self.scopes[scope].statement),
            common_scope: scope,
        });
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

trait Visitor {
    fn enter(&mut self, parent: usize, insertable: bool) -> usize;
    fn statement(&mut self, _index: usize, _scope: usize) {}
    fn simple(&mut self, _value: &mut Simple, _scope: usize) {}
    fn expression(&mut self, _value: &mut Expr, _scope: usize) {}
    fn finish(&mut self, _body: &mut Body, _scope: usize) {}
}
impl Visitor for Scan {
    fn enter(&mut self, parent: usize, insertable: bool) -> usize {
        let scope = self.scopes.len();
        self.scopes.push(Scope {
            parent,
            parent_statement: self.scopes[parent].statement,
            statement: 0,
            insertable,
        });
        scope
    }
    fn statement(&mut self, index: usize, scope: usize) {
        self.scopes[scope].statement = index;
    }
    fn simple(&mut self, value: &mut Simple, scope: usize) {
        if let Simple::Assign {
            binding,
            value: AssignmentValue::Value(value),
            ..
        } = value
        {
            // The initializer runs before the new binding takes its value.
            self.expression(value, scope);
            self.local(binding.local, scope, true);
        } else {
            evaluation::simple_accesses(value, |local| self.local(local, scope, false));
        }
    }
    fn expression(&mut self, value: &mut Expr, scope: usize) {
        evaluation::expression_accesses(value, |local| self.local(local, scope, false));
    }
}
struct Apply {
    assignments: IdVec<LocalId, bool>,
    declarations: Vec<Vec<(usize, Declaration)>>,
    scope: usize,
}
impl Visitor for Apply {
    fn enter(&mut self, _parent: usize, _insertable: bool) -> usize {
        let scope = self.scope;
        self.scope += 1;
        scope
    }
    fn simple(&mut self, value: &mut Simple, _scope: usize) {
        if let Simple::Assign {
            binding,
            value: AssignmentValue::Value(_),
            declaration,
        } = value
        {
            *declaration = std::mem::take(&mut self.assignments[binding.local]);
        }
    }
    fn finish(&mut self, body: &mut Body, scope: usize) {
        let mut declarations = std::mem::take(&mut self.declarations[scope]);
        if declarations.is_empty() {
            return;
        }
        // Positions refer to the completed tree scanned above. Stable ordering
        // keeps declarations at one site in their existing local order.
        declarations.sort_by_key(|(statement, _)| *statement);
        let statements = std::mem::take(body);
        body.reserve(statements.len() + declarations.len());
        let mut declarations = declarations.into_iter().peekable();
        for (index, statement) in statements.into_iter().enumerate() {
            while declarations.peek().is_some_and(|(at, _)| *at == index) {
                let (_, declaration) = declarations.next().unwrap();
                body.push(Statement::Simple(Simple::Declare {
                    binding: declaration.binding,
                    initialize: declaration.initialize,
                }));
            }
            body.push(statement);
        }
        debug_assert!(declarations.next().is_none());
    }
}

fn nested(body: &mut Body, parent: usize, visitor: &mut impl Visitor) {
    let scope = visitor.enter(parent, true);
    walk(body, scope, visitor);
}
fn walk(body: &mut Body, scope: usize, visitor: &mut impl Visitor) {
    for (index, statement) in body.iter_mut().enumerate() {
        visitor.statement(index, scope);
        match statement {
            Statement::Simple(value) => visitor.simple(value, scope),
            Statement::Return(Some(value)) => {
                visitor.expression(value, scope);
            }
            Statement::Throw(value) => visitor.expression(&mut value.value, scope),
            Statement::If { condition, .. } => visitor.expression(condition, scope),
            Statement::Loop { header, body, .. } => {
                match header {
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
                    }
                    LoopHeader::While(condition) => {
                        visitor.expression(condition, scope);
                        nested(body, scope, visitor);
                    }
                    LoopHeader::DoWhile(condition) => {
                        nested(body, scope, visitor);
                        visitor.expression(condition, scope);
                    }
                }
                continue;
            }
            Statement::Switch { value, .. } => visitor.expression(&mut value.value, scope),
            Statement::Synchronized { monitor, .. } => visitor.expression(monitor, scope),
            _ => (),
        }
        evaluation::children(statement, |body| nested(body, scope, visitor));
    }
    visitor.finish(body, scope);
}

#[cfg(test)]
#[path = "../../tests/unit/source_locals.rs"]
mod tests;
