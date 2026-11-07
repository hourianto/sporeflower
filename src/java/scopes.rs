//! Choose catch parameters from the final source scopes. A handler value may be
//! shared by several protected regions or leave a catch through a dispatcher;
//! only values whose uses stay in lexical catch bodies can live in parameters.
use crate::ir::scheduled::{self, ValueUse};
use crate::ir::{LocalId, MethodIr, Statement, ValueId};
use crate::structure::{FlowEnd, Stmt};
use std::collections::HashSet;

pub fn lexical_catches(ir: &MethodIr, body: &[Stmt]) -> HashSet<LocalId> {
    let candidates: HashSet<_> = ir
        .blocks
        .iter()
        .filter_map(|block| {
            block
                .caught_exception
                .and_then(|value| ir.values[value].local)
        })
        .collect();
    if candidates.is_empty() {
        return candidates;
    }
    let mut scan = Scan {
        ir,
        candidates,
        outside: HashSet::new(),
    };
    scan.body(body, &HashSet::new());
    scan.candidates
        .retain(|local| !scan.outside.contains(local));
    scan.candidates
}

struct Scan<'a> {
    ir: &'a MethodIr,
    candidates: HashSet<LocalId>,
    outside: HashSet<LocalId>,
}

impl Scan<'_> {
    fn local(&mut self, local: LocalId, scope: &HashSet<LocalId>) {
        if self.candidates.contains(&local) && !scope.contains(&local) {
            self.outside.insert(local);
        }
    }

    fn uses(&mut self, values: impl Iterator<Item = ValueUse>, scope: &HashSet<LocalId>) {
        for usage in values {
            if let Some(local) = usage.local(self.ir) {
                self.local(local, scope);
            } else if let Some((local, _)) = self.ir.values[usage.value()].kind.local_write() {
                self.local(local, scope);
            }
        }
    }

    fn value(&mut self, value: ValueId, scope: &HashSet<LocalId>) {
        self.uses(
            scheduled::values(self.ir, ValueUse::Reference(value)),
            scope,
        );
    }

    fn terminator(&mut self, head: crate::ir::BlockId, scope: &HashSet<LocalId>) {
        for value in self.ir.blocks[head].terminator.operands() {
            self.value(value, scope);
        }
    }

    fn body(&mut self, body: &[Stmt], scope: &HashSet<LocalId>) {
        for statement in body {
            match statement {
                Stmt::Basic(_) | Stmt::Statements { .. } => {
                    for (_, statement) in statement.scheduled(self.ir).unwrap().iter() {
                        if let Statement::Assign { local, .. } = statement {
                            self.local(*local, scope);
                        }
                        self.uses(scheduled::statement_values(self.ir, statement), scope);
                    }
                }
                Stmt::Return(head) | Stmt::Throw(head) => self.terminator(*head, scope),
                Stmt::If {
                    condition,
                    then_body,
                    else_body,
                    ..
                } => {
                    for head in condition.heads() {
                        self.terminator(head, scope);
                    }
                    self.body(then_body, scope);
                    self.body(else_body, scope);
                }
                Stmt::Loop {
                    body, continuation, ..
                } => {
                    self.body(body, scope);
                    if let Some(block) = continuation {
                        self.body(&[Stmt::Basic(*block)], scope);
                    }
                }
                Stmt::Label { body, .. } => self.body(body, scope),
                Stmt::Synchronized { monitor, body } => {
                    self.value(*monitor, scope);
                    self.body(body, scope);
                }
                Stmt::Switch { head, cases, .. } => {
                    self.terminator(*head, scope);
                    for case in cases {
                        self.body(&case.body, scope);
                    }
                }
                Stmt::Try { body, catches } => {
                    self.body(body, scope);
                    for catch in catches {
                        let mut scope = scope.clone();
                        if let Some(local) = self.ir.blocks[catch.handler]
                            .caught_exception
                            .and_then(|value| self.ir.values[value].local)
                        {
                            // Physical storage can be shared when the outer
                            // exception dies before a nested catch. Java's
                            // parameter scopes still overlap: use explicit
                            // catch-to-local assignments for this binding.
                            if !scope.insert(local) {
                                self.outside.insert(local);
                            }
                        }
                        self.body(&catch.body, &scope);
                    }
                }
                Stmt::TryFinally { body, cleanup, .. } => {
                    self.body(body, scope);
                    self.body(cleanup, scope);
                }
                Stmt::Dispatch { cases, .. } => {
                    for case in cases {
                        for handler in &case.handlers {
                            if let Some(local) = self.ir.blocks[handler.target]
                                .caught_exception
                                .and_then(|value| self.ir.values[value].local)
                            {
                                self.outside.insert(local);
                            }
                        }
                        self.body(&case.body, scope);
                        if !matches!(case.end, FlowEnd::Unreachable | FlowEnd::Multi(_)) {
                            self.terminator(case.head, scope);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
