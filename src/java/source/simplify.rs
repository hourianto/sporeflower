//! Coordinate source-local substitution, choice folding and loop presentation.
//! Shared evaluation facts and structured continuations constrain every rewrite.
use super::bound::{Body, Simple, Statement};
use super::evaluation::{
    Cost, Effects, Evaluation, Operation, add_counts, cost, effects, immediate, statement_effects,
    walk_body,
};
use super::lifetimes::{self, Flow, Live, Scope, flows, live_after};
use super::normalize;
use crate::{
    ir::LocalId,
    java::syntax::{Expr, Expression},
};
use std::collections::BTreeMap;

struct Substitute<'a> {
    local: LocalId,
    value: Option<Expr>,
    moving: &'a Effects,
    prefix: Effects,
    replaced: bool,
    fits: bool,
}

impl Evaluation for Substitute<'_> {
    fn target_read(&mut self, expression: &mut Expr, local: LocalId, conditional: bool) {
        self.prefix.read(expression, local, conditional);
    }
    fn read(&mut self, expression: &mut Expr, local: LocalId, conditional: bool) {
        if local == self.local
            && (!conditional || self.moving.discardable())
            && self.fits
            && self.moving.commutes(&self.prefix)
            && let Some(value) = self.value.take()
        {
            *expression = value;
            self.replaced = true;
        } else {
            self.prefix.read(expression, local, conditional);
        }
    }
    fn write(&mut self, local: LocalId) {
        self.prefix.write(local);
    }
    fn operation(&mut self, operation: Operation) {
        self.prefix.operation(operation);
    }
}

/// This pass never consults scheduled IR. Its facts describe the expressions
/// that Java will actually evaluate, including inserted conversions.
pub(super) fn simplify(body: &mut Body, result: &super::bound::ValueType) {
    normalize::body(body, result);
    let mut uses = Effects::default();
    walk_body(body, &mut uses);
    let mut accesses = BTreeMap::new();
    add_counts(&mut accesses, uses.occurrences());
    inline_body(
        body,
        &Live::new(),
        &Scope::default(),
        &mut accesses,
        result,
        false,
    );
}

fn inline_body(
    body: &mut Body,
    continuation: &Live,
    scope: &Scope,
    accesses: &mut BTreeMap<LocalId, usize>,
    result: &super::bound::ValueType,
    preserve: bool,
) {
    if body.is_empty() {
        return;
    }
    let mut owners: Vec<_> = body
        .iter()
        .map(|statement| preserve || super::choices::owns_ladder(statement, result))
        .collect();
    if body.iter().any(|statement| {
        matches!(
            statement,
            Statement::If { .. }
                | Statement::Loop { .. }
                | Statement::Switch { .. }
                | Statement::Block { .. }
                | Statement::Try { .. }
                | Statement::Synchronized { .. }
        )
    }) {
        let after = live_after(&flows(body, scope), continuation);
        for ((statement, after), owner) in body.iter_mut().zip(after).zip(&owners) {
            let alternative = matches!(statement, Statement::If { .. });
            lifetimes::nested_lifetimes(statement, &after, scope, |body, after, scope| {
                inline_body(body, after, scope, accesses, result, alternative && *owner);
            });
        }
    }
    loop {
        let mut changed = inline_bindings(body, &mut owners, continuation, scope, accesses);
        changed |= present_choices(body, &mut owners, result, accesses);
        changed |= present_loops(body, &mut owners, result, accesses);
        if !changed {
            break;
        }
    }
}

fn inline_bindings(
    body: &mut Body,
    owners: &mut Vec<bool>,
    continuation: &Live,
    scope: &Scope,
    accesses: &mut BTreeMap<LocalId, usize>,
) -> bool {
    if !body
        .windows(2)
        .any(|pair| inline_candidate(&pair[0], &pair[1]))
    {
        return false;
    }
    let mut changed = false;
    let mut summaries = flows(body, scope);
    let mut after = live_after(&summaries, continuation);
    let mut index = 0;
    while index + 1 < body.len() {
        if !inline_candidate(&body[index], &body[index + 1]) {
            index += 1;
            continue;
        }
        let Statement::Simple(Simple::Assign { binding, value, .. }) = &body[index] else {
            unreachable!("inline candidate is an assignment")
        };
        let local = binding.local;
        let mut replacement = binding.replacement(binding.assigned_value(value));
        let replacement_cost = cost(&mut replacement);
        if replacement_cost.nodes > 48 {
            index += 1;
            continue;
        }
        let moving = effects(&mut replacement);
        let mut substitute = Substitute {
            local,
            value: Some(replacement),
            moving: &moving,
            prefix: Effects::default(),
            replaced: false,
            fits: true,
        };
        let mut consumer = None;
        for (offset, statement) in body[index + 1..].iter_mut().enumerate() {
            let next = index + offset + 1;
            substitute.fits =
                summaries[next].reads.get(&local) == Some(&1) && !after[next].contains(&local);
            if substitute.fits {
                let mut size = Cost::default();
                immediate(statement, &mut size);
                substitute.fits = replacement_cost.fits_into(&size);
            }
            immediate(statement, &mut substitute);
            if substitute.replaced {
                consumer = Some(next);
            }
            if substitute.replaced
                || !moving.commutes(&substitute.prefix)
                || !matches!(statement, Statement::Simple(_))
                || substitute.prefix.reads.contains_key(&local)
                || substitute.prefix.writes.contains_key(&local)
            {
                break;
            }
        }
        if let Some(consumer) = consumer {
            summaries[consumer] = Flow::of(&mut body[consumer], scope);
            subtract_counts(accesses, [(local, 2)]);
            body.remove(index);
            summaries.remove(index);
            owners.remove(index);
            changed = true;
            after = live_after(&summaries, continuation);
        } else {
            index += 1;
        }
    }
    changed
}

fn inline_candidate(statement: &Statement, next: &Statement) -> bool {
    let Statement::Simple(Simple::Assign { binding, .. }) = statement else {
        return false;
    };
    if binding.synthetic {
        return true;
    }
    // Debug names explain values consumed by other calculations. Direct
    // returns and monitor captures already have a clear source role.
    let value = match next {
        Statement::Return(Some(value)) | Statement::Synchronized { monitor: value, .. } => value,
        _ => return false,
    };
    let mut value = value;
    loop {
        match &value.kind {
            Expression::Local { id, .. } => return *id == binding.local,
            Expression::Group(inner)
            | Expression::Commented(inner, _)
            | Expression::Cast { value: inner, .. } => value = inner,
            _ => return false,
        }
    }
}

/// Choice folding consumes a statement and possibly its adjacent guard tail.
/// Maintain only those occurrence deltas; finalized child scopes are not run
/// again when their parent exposes a new value-producing assignment.
fn present_choices(
    body: &mut Body,
    owners: &mut Vec<bool>,
    result: &super::bound::ValueType,
    accesses: &mut BTreeMap<LocalId, usize>,
) -> bool {
    let mut output = Vec::new();
    let mut output_owners = Vec::new();
    let mut summaries: Vec<Effects> = Vec::new();
    let mut changed = false;
    for (mut statement, owner) in std::mem::take(body).into_iter().zip(std::mem::take(owners)) {
        let original = statement_effects(&mut statement);
        super::choices::append_bound(&mut output, statement, result, owner);
        subtract_counts(accesses, original.occurrences());
        let mut removed_statements = original.statements;
        while summaries.len() >= output.len() {
            let removed = summaries.pop().unwrap();
            subtract_counts(accesses, removed.occurrences());
            removed_statements += removed.statements;
        }
        output_owners.truncate(output.len() - 1);
        output_owners.push(owner);
        let tail = output.last_mut().unwrap();
        normalize::statement(tail, result);
        let summary = statement_effects(tail);
        changed |= summary.statements < removed_statements
            || original.reads != summary.reads
            || original.writes != summary.writes;
        add_counts(accesses, summary.occurrences());
        summaries.push(summary);
    }
    *body = output;
    *owners = output_owners;
    changed
}

fn present_loops(
    body: &mut Body,
    owners: &mut Vec<bool>,
    result: &super::bound::ValueType,
    accesses: &mut BTreeMap<LocalId, usize>,
) -> bool {
    let loop_accesses = accesses.clone();
    let mut output = Vec::new();
    let mut output_owners = Vec::new();
    let mut changed = false;
    for (mut statement, owner) in std::mem::take(body).into_iter().zip(std::mem::take(owners)) {
        let original = statement_effects(&mut statement);
        subtract_counts(accesses, original.occurrences());
        if !super::completion::can_complete(&output) {
            changed = true;
            continue;
        }
        let mut statements = 0;
        for mut statement in super::loops::present_statement(statement, &loop_accesses) {
            normalize::statement(&mut statement, result);
            let summary = statement_effects(&mut statement);
            statements += summary.statements;
            add_counts(accesses, summary.occurrences());
            output_owners.push(owner || super::choices::owns_ladder(&statement, result));
            output.push(statement);
        }
        changed |= statements < original.statements;
    }
    *body = output;
    *owners = output_owners;
    changed
}

fn subtract_counts(
    counts: &mut BTreeMap<LocalId, usize>,
    removed: impl IntoIterator<Item = (LocalId, usize)>,
) {
    for (local, count) in removed {
        let previous = counts
            .get_mut(&local)
            .expect("source occurrence missing from method summary");
        *previous -= count;
        if *previous == 0 {
            counts.remove(&local);
        }
    }
}
