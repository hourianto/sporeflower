//! Bind Java's first constructor invocation against the same names, overloads
//! and local identities as its body. Only pure copies can be expressed again
//! as arguments and left at the start of the body after delegation.
use super::{Body, Statement, bind::Binder, bound::Simple, evaluation};
use crate::{
    Result,
    ir::{LocalId, Statement as Scheduled},
    java::{
        local_flow::statement_reads,
        syntax::{Expr, Expression},
    },
};
use anyhow::ensure;
use std::collections::{BTreeMap, BTreeSet};

pub(in crate::java) struct ConstructorEntry<'a> {
    pub invocation: Option<&'a Scheduled>,
    pub crossed: Vec<&'a Scheduled>,
    pub delegates: bool,
    pub defaults: &'a [Expr],
}

impl ConstructorEntry<'_> {
    pub(super) fn bind(&self, binder: &Binder<'_, '_, '_>) -> Result<Body> {
        let mut entry = Vec::new();
        let ir = binder.expr.ir;
        let mut copies = BTreeMap::new();
        let mut delayed = BTreeSet::new();
        for &statement in &self.crossed {
            delayed.extend(statement_reads(ir, statement));
            let Scheduled::Assign { local, .. } = statement else {
                // Receiver stores have already been proved movable across the parent.
                continue;
            };
            ensure!(
                delayed.insert(*local),
                "constructor prefix overwrites a live local"
            );
            let Simple::Assign { binding, value, .. } = binder.scheduled(statement)? else {
                anyhow::bail!("constructor prefix requires an update");
            };
            let mut value = binding.assigned_value(&value);
            ensure!(
                evaluation::effects(&mut value).discardable(),
                "constructor prefix has an observable assignment"
            );
            substitute(&mut value, &copies)?;
            copies.insert(*local, binding.replacement(value));
        }
        if let Some(call) = self.invocation {
            let Simple::Delegation(mut invocation) = binder.scheduled(call)? else {
                unreachable!("constructor invocation is a bound expression")
            };
            let Expression::Invocation { arguments, .. } = &mut invocation.kind else {
                unreachable!("bound delegation is an invocation")
            };
            for argument in arguments {
                ensure!(
                    evaluation::effects(argument)
                        .writes
                        .keys()
                        .all(|&local| ir.locals[local].parameter.is_some()
                            && !delayed.contains(&local)),
                    "delegation writes a local needed by its prefix"
                );
                substitute(argument, &copies)?;
                // A pure copy of this may remain in the body after super().
                // Only an actual delegation argument, after substitution of
                // its full chain of copies, must be available before super().
                let mut available = true;
                evaluation::visit_expression(argument, |node| match &node.kind {
                    Expression::Local { id, .. } => available &= ir.locals[*id].parameter.is_some(),
                    Expression::Atom(name) if name == "this" => available = false,
                    _ => (),
                });
                ensure!(
                    available,
                    "delegation depends on an uninitialized receiver or body local"
                );
            }
            entry.push(Statement::Simple(Simple::Delegation(invocation)));
        }
        if !self.delegates {
            entry.extend(
                self.defaults
                    .iter()
                    .cloned()
                    .map(|value| Statement::Simple(Simple::Expression(value))),
            );
        }
        Ok(entry)
    }
}

/// Binding::replacement keeps the original Java operand type, including a
/// necessary cast at an overloaded call. A substitution replaces only a read;
/// the caller rejects writes to any crossed binding before visiting it.
fn substitute(value: &mut Expr, copies: &BTreeMap<LocalId, Expr>) -> Result<()> {
    let mut expanded = false;
    evaluation::visit_expression(value, |node| {
        if let Expression::Local { id, .. } = &node.kind
            && let Some(replacement) = copies.get(id)
        {
            *node = replacement.clone();
            expanded = true;
        }
    });
    ensure!(
        !expanded || evaluation::expression_fits(value),
        "constructor argument exceeds source expansion budget"
    );
    Ok(())
}
