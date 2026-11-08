//! Structured source liveness includes normal continuations, protected handler
//! observations and labeled transfers. Nested rewrites receive those same scopes.
use super::bound::{Body, Simple, Statement};
use super::evaluation::{Effects, add_counts, evaluate_header, evaluations};
use crate::ir::LocalId;
use std::collections::{BTreeMap, BTreeSet};

pub(super) type Live = BTreeSet<LocalId>;

#[derive(Clone, Default)]
pub(super) struct Scope {
    observers: Live,
    breaks: Vec<(Option<String>, Live)>,
    continues: Vec<(Option<String>, Live)>,
}

impl Scope {
    fn with_break(&self, label: &Option<String>, after: &Live) -> Self {
        let mut scope = self.clone();
        scope.breaks.push((None, after.clone()));
        if label.is_some() {
            scope.breaks.push((label.clone(), after.clone()));
        }
        scope
    }

    fn with_loop(&self, label: &Option<String>, after: &Live, repeated: &Live) -> Self {
        let mut scope = self.with_break(label, after);
        scope.continues.push((None, repeated.clone()));
        if label.is_some() {
            scope.continues.push((label.clone(), repeated.clone()));
        }
        scope
    }
}

/// A source statement's transfer is cached once per body. Definite simple
/// stores kill the preceding value; compound control keeps conservative reads.
/// Rewriting one expression refreshes only that statement's transfer.
pub(super) struct Flow {
    pub(super) reads: BTreeMap<LocalId, usize>,
    incoming: Live,
    kills: Live,
    escapes: Live,
    completes: bool,
}

impl Flow {
    pub(super) fn of(statement: &mut Statement, scope: &Scope) -> Self {
        let mut accesses = Effects::default();
        evaluations(statement, &mut accesses);
        let mut incoming = accesses.reads.keys().copied().collect::<Live>();
        let mut kills = match statement {
            Statement::Simple(
                Simple::Assign { binding, .. }
                | Simple::Declare {
                    binding,
                    initialize: true,
                },
            ) => Live::from([binding.local]),
            _ => Live::new(),
        };
        // A lexical child may define its own capture before reading it. Keep
        // total occurrences for substitution, but expose only entry reads to
        // the enclosing statement and to the next loop iteration.
        let mut summarize = |body: &mut Body, scope: &Scope| {
            let mut summary = sequence(body, scope);
            add_counts(&mut accesses.reads, std::mem::take(&mut summary.reads));
            summary
        };
        match statement {
            Statement::If { yes, no, .. } => {
                let yes = summarize(yes, scope);
                let no = summarize(no, scope);
                incoming.extend(&yes.incoming);
                incoming.extend(&no.incoming);
                kills = completing_writes([&yes, &no]);
            }
            Statement::Loop { label, body, .. } => {
                let nested = scope.with_loop(label, &Live::new(), &Live::new());
                incoming.extend(summarize(body, &nested).incoming);
            }
            Statement::Switch { label, cases, .. } => {
                let nested = scope.with_break(label, &Live::new());
                for case in cases {
                    incoming.extend(summarize(&mut case.body, &nested).incoming);
                }
            }
            Statement::Block { label, body } => {
                let mut nested = scope.clone();
                if label.is_some() {
                    nested.breaks.push((label.clone(), Live::new()));
                }
                let summary = summarize(body, &nested);
                incoming.extend(summary.incoming);
                if label.is_none() {
                    kills = summary.kills;
                }
            }
            Statement::Synchronized { body, .. } => {
                let summary = summarize(body, scope);
                incoming.extend(summary.incoming);
                kills = summary.kills;
            }
            Statement::Try {
                body,
                catches,
                cleanup,
            } => {
                let mut paths = vec![summarize(body, scope)];
                for catch in catches {
                    let mut path = summarize(&mut catch.body, scope);
                    if let Some(local) = catch.local {
                        path.incoming.remove(&local);
                        path.kills.insert(local);
                    }
                    paths.push(path);
                }
                for path in &paths {
                    incoming.extend(&path.incoming);
                }
                kills = completing_writes(paths.iter());
                if let Some(cleanup) = cleanup {
                    let cleanup = summarize(cleanup, scope);
                    incoming.extend(cleanup.incoming);
                    kills.extend(cleanup.kills);
                }
            }
            _ => (),
        }
        let mut escapes = scope.observers.clone();
        transfer_reads(statement, scope, &mut escapes);
        Self {
            reads: accesses.reads,
            incoming,
            kills,
            escapes,
            completes: completes(statement),
        }
    }

    fn before(&self, live: &mut Live) {
        if !self.completes {
            live.clear();
        }
        live.retain(|local| !self.kills.contains(local));
        live.extend(&self.incoming);
        live.extend(&self.escapes);
    }
}

fn sequence(body: &mut Body, scope: &Scope) -> Flow {
    let mut summary = Flow {
        reads: BTreeMap::new(),
        incoming: Live::new(),
        kills: Live::new(),
        escapes: Live::new(),
        completes: true,
    };
    let mut reachable = true;
    for statement in body {
        let transfer = Flow::of(statement, scope);
        // Occurrences include every statement. Entry reads only come from
        // reachable evaluations that precede a definite store to that local.
        add_counts(&mut summary.reads, transfer.reads);
        summary.completes = transfer.completes;
        if reachable {
            let killed = &summary.kills;
            summary.incoming.extend(
                transfer
                    .incoming
                    .into_iter()
                    .chain(transfer.escapes)
                    .filter(|local| !killed.contains(local)),
            );
            summary.kills.extend(transfer.kills);
            reachable = transfer.completes;
        }
    }
    summary
}

fn completing_writes<'a>(paths: impl IntoIterator<Item = &'a Flow>) -> Live {
    let mut paths = paths.into_iter().filter(|path| path.completes);
    let Some(first) = paths.next() else {
        return Live::new();
    };
    let mut kills = first.kills.clone();
    for path in paths {
        kills.retain(|local| path.kills.contains(local));
    }
    kills
}

fn completes(statement: &Statement) -> bool {
    super::completion::can_complete(std::slice::from_ref(statement))
}

fn transfer_reads(statement: &Statement, scope: &Scope, result: &mut Live) {
    for (continuing, targets) in [(false, &scope.breaks), (true, &scope.continues)] {
        if targets.iter().all(|(_, live)| live.is_empty()) {
            continue;
        }
        let mut seen = BTreeSet::new();
        for (label, live) in targets.iter().rev() {
            if seen.insert(label)
                && !live.is_empty()
                && super::completion::transfers_to(
                    std::slice::from_ref(statement),
                    label,
                    label.is_none(),
                    continuing,
                )
            {
                result.extend(live);
            }
        }
    }
}

pub(super) fn flows(body: &mut Body, scope: &Scope) -> Vec<Flow> {
    body.iter_mut()
        .map(|statement| Flow::of(statement, scope))
        .collect()
}

fn live_before(flows: &[Flow], live: &mut Live) {
    for flow in flows.iter().rev() {
        flow.before(live);
    }
}

pub(super) fn live_after(flows: &[Flow], continuation: &Live) -> Vec<Live> {
    let mut live = continuation.clone();
    let mut after = vec![Live::new(); flows.len()];
    for (index, flow) in flows.iter().enumerate().rev() {
        after[index] = if flow.completes {
            live.clone()
        } else {
            Live::new()
        };
        after[index].extend(&flow.escapes);
        flow.before(&mut live);
    }
    after
}

pub(super) fn nested_lifetimes(
    statement: &mut Statement,
    after: &Live,
    scope: &Scope,
    mut visit: impl FnMut(&mut Body, &Live, &Scope),
) {
    match statement {
        Statement::If { yes, no, .. } => {
            visit(yes, after, scope);
            visit(no, after, scope);
        }
        Statement::Loop {
            label,
            header,
            body,
        } => {
            let nested = scope.with_loop(label, after, &Live::new());
            let mut repeated = sequence(body, &nested).incoming;
            let mut header_reads = Effects::default();
            evaluate_header(header, &mut header_reads);
            repeated.extend(header_reads.reads.into_keys());
            repeated.extend(after);
            let nested = scope.with_loop(label, after, &repeated);
            visit(body, &repeated, &nested);
        }
        Statement::Switch { label, cases, .. } => {
            let nested = scope.with_break(label, after);
            let mut next = after.clone();
            for case in cases.iter_mut().rev() {
                visit(&mut case.body, &next, &nested);
                live_before(&flows(&mut case.body, &nested), &mut next);
            }
        }
        Statement::Block { label, body } => {
            let mut nested = scope.clone();
            if label.is_some() {
                nested.breaks.push((label.clone(), after.clone()));
            }
            visit(body, after, &nested);
        }
        Statement::Synchronized { body, .. } => visit(body, after, scope),
        Statement::Try {
            body,
            catches,
            cleanup,
        } => {
            let mut protected = scope.clone();
            let mut continuation = after.clone();
            if let Some(cleanup) = cleanup {
                visit(cleanup, after, scope);
                let cleanup = sequence(cleanup, scope);
                cleanup.before(&mut continuation);
                cleanup.before(&mut protected.observers);
                for (_, live) in protected.breaks.iter_mut().chain(&mut protected.continues) {
                    cleanup.before(live);
                }
            }
            for catch in catches.iter_mut() {
                visit(&mut catch.body, &continuation, &protected);
            }
            // A handler can observe storage through its continuation without
            // reading it itself. Its entry transfer also preserves enclosing
            // break/continue targets and runs any cleanup before those exits.
            let mut observers = protected.observers.clone();
            for catch in catches {
                let mut entry = continuation.clone();
                live_before(&flows(&mut catch.body, &protected), &mut entry);
                if let Some(local) = catch.local {
                    entry.remove(&local);
                }
                observers.extend(entry);
            }
            protected.observers = observers;
            visit(body, &continuation, &protected);
        }
        _ => (),
    }
}
