//! Choose catch parameters from the final source scopes. A handler value may be
//! shared by several protected regions or leave a catch through a dispatcher;
//! only values whose uses stay in lexical catch bodies can live in parameters.
use super::expr::ExprPlanner;
use crate::hash::{HashMap, HashSet};
use crate::ir::scheduled::{self, ValueUse};
use crate::ir::{LocalId, MethodIr, Statement, ValueId};
use crate::structure::{FlowEnd, Stmt, tree::Operation};
use std::collections::BTreeSet;

pub(super) fn lexical_catches(
    expr: &mut ExprPlanner<'_, '_>,
    body: &[Stmt],
    selected: Option<&BTreeSet<LocalId>>,
) -> HashSet<LocalId> {
    let ir = expr.ir;
    let candidates: HashSet<_> = ir
        .blocks
        .iter()
        .filter_map(|block| {
            block
                .caught_exception
                .and_then(|value| ir.values[value].local)
        })
        .filter(|local| selected.is_none_or(|set| set.contains(local)))
        .collect();
    if candidates.is_empty() {
        return candidates;
    }
    let mut scan = Scan {
        ir,
        candidates,
        depths: HashMap::default(),
    };
    scan.body(body, &HashSet::default());
    scan.name_disjoint_catches(expr);
    scan.candidates
}

struct Scan<'a> {
    ir: &'a MethodIr,
    candidates: HashSet<LocalId>,
    depths: HashMap<LocalId, Option<usize>>,
}

impl Scan<'_> {
    fn name_disjoint_catches(&self, expr: &mut ExprPlanner<'_, '_>) {
        if self.depths.len() < 2 {
            return;
        }
        // Reuse only spellings already allocated to these lexical parameters.
        // Fields, ordinary locals, debug names and renderer-generated bindings
        // keep their reservations. A handler reused at two depths keeps its name.
        let mut eligible: Vec<_> = self
            .depths
            .iter()
            .filter_map(|(&id, &depth)| {
                let local = &self.ir.locals[id];
                let name = expr.local_name(id);
                (self.candidates.contains(&id)
                    && local.name_hint.is_none()
                    && !local.name.starts_with('$')
                    && name == local.name
                    && local.parameter.is_none()
                    && !local.is_this)
                    .then_some((id, depth?, name))
            })
            .collect();
        if eligible.len() < 2 {
            return;
        }
        let mut spellings: Vec<_> = eligible.iter().map(|&(_, _, name)| name).collect();
        spellings.sort_unstable_by_key(|name| (name.len(), *name));
        eligible.sort_unstable_by_key(|&(_, depth, _)| depth);
        let mut updates = Vec::new();
        for (group, &name) in eligible.chunk_by(|a, b| a.1 == b.1).zip(&spellings) {
            updates.extend(
                group
                    .iter()
                    .filter(|entry| entry.2 != name)
                    .map(|entry| (entry.0, name.to_owned())),
            );
        }
        expr.rename_locals(updates);
    }
    fn local(&mut self, local: LocalId, scope: &HashSet<LocalId>) {
        if !scope.contains(&local) {
            self.candidates.remove(&local);
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
            let ir = self.ir;
            statement.operations(ir, &mut |operation| match operation {
                Operation::Statement { statement, .. } => {
                    if let Statement::Assign { local, .. } = statement {
                        self.local(*local, scope);
                    }
                    self.uses(scheduled::statement_values(ir, statement), scope);
                }
                Operation::Terminator { block, .. } => self.terminator(block, scope),
                Operation::Expression(value) => self.value(value, scope),
                Operation::CatchBinding(_) => (),
            });
            match statement {
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
                            let depth = scope.len();
                            self.depths
                                .entry(local)
                                .and_modify(|previous| {
                                    if *previous != Some(depth) {
                                        *previous = None;
                                    }
                                })
                                .or_insert(Some(depth));
                            if !scope.insert(local) {
                                self.candidates.remove(&local);
                            }
                        }
                        self.body(&catch.body, &scope);
                    }
                }
                Stmt::Dispatch { cases, .. } => {
                    for case in cases {
                        for handler in &case.handlers {
                            if let Some(local) = self.ir.blocks[handler.target]
                                .caught_exception
                                .and_then(|value| self.ir.values[value].local)
                            {
                                self.candidates.remove(&local);
                            }
                        }
                        self.body(&case.body, scope);
                        if !matches!(case.end, FlowEnd::Unreachable | FlowEnd::Multi(_)) {
                            self.terminator(case.head, scope);
                        }
                    }
                }
                // Opaque BoundRegion exits were never lexical catch bodies.
                Stmt::BoundRegion { .. } => (),
                _ => {
                    for body in statement.bodies() {
                        self.body(body, scope);
                    }
                }
            }
        }
    }
}
