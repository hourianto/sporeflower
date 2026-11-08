//! Source choices have one presentation owner over complete bound expressions.
//! During lowering, default initialization also needs scheduled identities and
//! handler observations; it does not choose expressions or statement polarity.
use super::{Body, Statement, bound, evaluation};
use crate::ir::{LocalId, MethodIr};
use crate::java::syntax::{
    Expr, Expression as BoundExpression, Operator, Prefix, SourceLeaf as Baseline,
};
use bound::{AssignmentValue, Simple};
use std::{
    cell::OnceCell,
    collections::{BTreeMap, BTreeSet},
};

const MAX_CHOICE_DEPTH: usize = 3;

/// Default preparation examines a complete decision tree once. Handler
/// observations include failed analysis, avoiding repeated whole-method proofs.
pub(super) struct Defaults {
    handler_observers: OnceCell<Option<BTreeSet<LocalId>>>,
}

impl Defaults {
    pub(super) fn new() -> Self {
        Self {
            handler_observers: OnceCell::new(),
        }
    }

    fn handler_observers(&self, ir: &MethodIr) -> Option<&BTreeSet<LocalId>> {
        self.handler_observers
            .get_or_init(|| {
                let handlers: BTreeSet<_> = ir
                    .blocks
                    .iter()
                    .flat_map(|block| block.handlers.iter().map(|handler| handler.target))
                    .collect();
                if handlers.is_empty() {
                    return Some(BTreeSet::new());
                }
                let live = crate::ir::scheduled_liveness::LocalLiveness::analyze(ir).ok()?;
                Some(
                    handlers
                        .into_iter()
                        .flat_map(|handler| live.at_handler(ir, handler))
                        .collect(),
                )
            })
            .as_ref()
    }

    pub(super) fn append(
        &self,
        body: &mut Body,
        mut statement: Statement,
        binder: &super::bind::Binder<'_, '_, '_>,
    ) {
        let expr = binder.expr;
        match default_binding(expr.ir, &statement, self) {
            DefaultPlan::Initialize(destination, baseline, value) => {
                body.push(Statement::Simple(binder.assignment(
                    destination,
                    value,
                    false,
                )));
                statement = prune(statement, baseline);
            }
            DefaultPlan::Separate => {
                // Lowering defers immediate If arms so one complete decision
                // can own its default. An enclosing branch with other work is
                // a scope containing independent decisions, not their owner.
                if let Statement::If { yes, no, .. } = &mut statement {
                    for arm in [yes, no] {
                        for child in std::mem::take(arm) {
                            self.append(arm, child, binder);
                        }
                    }
                }
            }
            DefaultPlan::Retain => (),
        }
        body.push(statement);
    }
}

/// Inspect the complete tree before its children choose their presentation.
/// Storage decisions retain their ladder; a single diamond remains an expression.
pub(super) fn owns_ladder(statement: &bound::Statement, result: &bound::ValueType) -> bool {
    let mut pending = vec![statement];
    let mut destination = None;
    let mut decisions = 0;
    let mut storage = false;
    let mut boolean = result.ty == crate::classfile::JvmType::Boolean;
    while let Some(statement) = pending.pop() {
        let target = match statement {
            bound::Statement::If { yes, no, .. } if yes.len() == 1 && no.len() == 1 => {
                decisions += 1;
                pending.extend([&yes[0], &no[0]]);
                continue;
            }
            bound::Statement::Simple(bound::Simple::Assign { binding, .. }) => {
                storage |= binding.storage;
                boolean = binding.ty == crate::classfile::JvmType::Boolean;
                Some(binding.local)
            }
            bound::Statement::Return(Some(_)) => None,
            _ => return false,
        };
        if destination.is_some_and(|previous| previous != target) {
            return false;
        }
        destination = Some(target);
    }
    destination.is_some() && !boolean && decisions > if storage { 1 } else { MAX_CHOICE_DEPTH }
}

pub(super) fn append_bound(
    body: &mut bound::Body,
    statement: bound::Statement,
    result: &bound::ValueType,
    preserve: bool,
) {
    let mut statement = match statement {
        bound::Statement::If {
            condition,
            mut yes,
            no,
        } if !preserve && yes.len() == 1 && no.len() == 1 => {
            let candidate = match (&yes[0], &no[0]) {
                (bound::Statement::Return(Some(a)), bound::Statement::Return(Some(b))) => {
                    choose(&condition, a, b, result)
                }
                (
                    bound::Statement::Simple(bound::Simple::Assign {
                        binding: a,
                        value: av,
                        declaration: false,
                    }),
                    bound::Statement::Simple(bound::Simple::Assign {
                        binding: b,
                        value: bv,
                        declaration: false,
                    }),
                ) if a.local == b.local => choose(
                    &condition,
                    &a.assigned_value(av),
                    &b.assigned_value(bv),
                    &a.value_type(),
                ),
                _ => None,
            };
            if let Some(value) = candidate {
                match yes.pop().unwrap() {
                    bound::Statement::Return(_) => bound::Statement::Return(Some(value)),
                    bound::Statement::Simple(bound::Simple::Assign { binding, .. }) => {
                        bound::Statement::Simple(bound::Simple::Assign {
                            binding,
                            value: bound::AssignmentValue::Value(value),
                            declaration: false,
                        })
                    }
                    _ => unreachable!(),
                }
            } else {
                bound::Statement::If { condition, yes, no }
            }
        }
        statement => statement,
    };
    if let bound::Statement::If { condition, yes, no } = &mut statement
        && !yes.is_empty()
        && !no.is_empty()
        && !matches!(no.as_slice(), [bound::Statement::If { .. }])
        && prefer_inverse(condition)
    {
        let original = std::mem::replace(condition, Expr::boolean(false));
        *condition = original.negated_test();
        std::mem::swap(yes, no);
    }
    if let bound::Statement::Return(Some(value)) = statement {
        // Consider the entire adjacent sequence, never an arbitrary short tail
        // of a large lookup. The final expression shares the inliner's budget.
        let guards: Vec<_> = body.iter().rev().map_while(return_guard).collect();
        if guards.is_empty() || preserve {
            body.push(bound::Statement::Return(Some(value)));
            return;
        }
        let mut candidate = value.clone();
        let mut fits = true;
        for (condition, yes) in &guards {
            if let Some(value) = choose(condition, yes, &candidate, result) {
                candidate = value;
            } else {
                fits = false;
                break;
            }
        }
        if fits {
            body.truncate(body.len() - guards.len());
            body.push(bound::Statement::Return(Some(candidate)));
        } else {
            body.push(bound::Statement::Return(Some(value)));
        }
    } else {
        body.push(statement);
    }
}

fn return_guard(statement: &bound::Statement) -> Option<(&Expr, &Expr)> {
    if let bound::Statement::If { condition, yes, no } = statement
        && no.is_empty()
        && let [bound::Statement::Return(Some(value))] = yes.as_slice()
    {
        Some((condition, value))
    } else {
        None
    }
}

fn choose(condition: &Expr, yes: &Expr, no: &Expr, result: &bound::ValueType) -> Option<Expr> {
    let logical = result.ty == crate::classfile::JvmType::Boolean
        && (yes.boolean_constant().is_some() && no.canonical_boolean()
            || no.boolean_constant().is_some() && yes.canonical_boolean());
    if !logical
        && [condition, yes, no]
            .into_iter()
            .any(|value| evaluation::expression_mutates(&mut value.clone()))
    {
        return None;
    }
    let mut value = if logical {
        match (yes.boolean_constant(), no.boolean_constant()) {
            (Some(a), Some(b)) if a != b => {
                if a {
                    condition.clone().truth_value()
                } else {
                    condition.clone().negated_test().truth_value()
                }
            }
            (Some(value), _) => Expr::logical(
                !value,
                vec![
                    if value {
                        condition.clone()
                    } else {
                        condition.clone().negated_test()
                    },
                    no.clone(),
                ],
            ),
            (_, Some(value)) => Expr::logical(
                !value,
                vec![
                    if value {
                        condition.clone().negated_test()
                    } else {
                        condition.clone()
                    },
                    yes.clone(),
                ],
            ),
            _ => unreachable!(),
        }
    } else {
        Expr::conditional(
            condition.clone(),
            result.operand(yes.clone()),
            result.operand(no.clone()),
        )
    };
    evaluation::expression_fits(&mut value).then_some(value)
}

fn prefer_inverse(condition: &Expr) -> bool {
    let BoundExpression::Logical { and: false, parts } = &condition.kind else {
        return false;
    };
    let mut negative = [0, 0];
    let mut pending: Vec<_> = parts.iter().collect();
    while let Some(value) = pending.pop() {
        match &value.kind {
            BoundExpression::Logical { parts, .. } => pending.extend(parts),
            BoundExpression::Group(value) | BoundExpression::Commented(value, _) => {
                pending.push(value);
            }
            BoundExpression::Prefix(Prefix::Not, _)
            | BoundExpression::Binary(Operator::NotEqual, _) => negative[0] += 1,
            // Neither direction of an ordered comparison has positive polarity.
            BoundExpression::Binary(
                Operator::Less | Operator::LessEqual | Operator::Greater | Operator::GreaterEqual,
                _,
            ) => (),
            _ => negative[1] += 1,
        }
    }
    negative[1] < negative[0]
}

fn baseline(expression: &Expr) -> Option<Baseline> {
    match expression.kind {
        BoundExpression::Local { id, .. } => Some(Baseline::Local(id)),
        _ => expression.facts.details.as_ref()?.origin,
    }
}

/// A complete assignment decision is analyzed once, even when its mutation or
/// handler proof rejects factoring. Only surrounding scopes expose independent
/// child decisions; retrying parts of a rejected tree would change its owner.
enum DefaultPlan {
    Separate,
    Retain,
    Initialize(LocalId, Baseline, Expr),
}

fn default_binding(ir: &MethodIr, statement: &Statement, planner: &Defaults) -> DefaultPlan {
    if !matches!(statement, Statement::If { .. }) {
        return DefaultPlan::Separate;
    }
    let mut destination = None;
    let mut leaves = Vec::new();
    let mut conditions = Vec::new();
    let mut initializers = Vec::new();
    let mut pending = vec![(std::slice::from_ref(statement), 0, false)];
    while let Some((body, depth, initialized)) = pending.pop() {
        // Pruning the accepted tree below is recursive. Very deep or wide
        // bytecode decisions retain their original statements.
        if depth > 64 || leaves.len() + conditions.len() > 4096 {
            return DefaultPlan::Retain;
        }
        let Some((statement, rest)) = body.split_first() else {
            if !initialized {
                return DefaultPlan::Separate;
            }
            continue;
        };
        match statement {
            Statement::If { condition, yes, no } if rest.is_empty() => {
                conditions.push(condition);
                pending.extend([
                    (yes.as_slice(), depth + 1, initialized),
                    (no.as_slice(), depth + 1, initialized),
                ]);
            }
            Statement::Simple(Simple::Assign {
                binding,
                value: AssignmentValue::Value(value),
                declaration: false,
            }) => {
                if destination.is_some_and(|previous| previous != binding.local) {
                    return DefaultPlan::Separate;
                }
                destination = Some(binding.local);
                if !rest.is_empty() {
                    initializers.push(value);
                }
                leaves.push(value);
                pending.push((rest, depth + 1, true));
            }
            _ => return DefaultPlan::Separate,
        }
    }
    let Some(destination) = destination else {
        return DefaultPlan::Separate;
    };
    let mut baselines = BTreeMap::<Baseline, (usize, &Expr)>::new();
    for value in &leaves {
        if let Some(source) = baseline(value)
            && source != Baseline::Local(destination)
        {
            baselines.entry(source).or_insert((0, *value)).0 += 1;
        }
    }
    let Some((&default, &(_, value))) = baselines.iter().find(|(_, (count, _))| *count >= 2) else {
        return DefaultPlan::Retain;
    };
    // An arbitrary earlier assignment followed by a reset to baseline must
    // keep both writes in order. Only a baseline initializer can move out.
    if initializers
        .iter()
        .any(|value| baseline(value) != Some(default))
        || leaves.iter().all(|value| baseline(value) == Some(default))
        || leaves
            .iter()
            .any(|value| depends_or_mutates(value, destination))
        || conditions
            .iter()
            .any(|value| depends_or_mutates(value, destination))
    {
        return DefaultPlan::Retain;
    }
    // Initializing the result earlier must not change a catch's view if a
    // condition or an arm throws before its original result assignment.
    if planner
        .handler_observers(ir)
        .is_none_or(|locals| locals.contains(&destination))
    {
        return DefaultPlan::Retain;
    }
    DefaultPlan::Initialize(destination, default, value.clone())
}

fn prune(statement: Statement, default: Baseline) -> Statement {
    fn arm(body: Body, default: Baseline) -> Body {
        body.into_iter().filter_map(|statement| {
            (!matches!(&statement, Statement::Simple(Simple::Assign { value: AssignmentValue::Value(value), .. }) if baseline( value) == Some(default)))
                .then(|| prune(statement, default))
        }).collect()
    }
    let (mut condition, mut yes, mut no) = match statement {
        Statement::If { condition, yes, no } => (condition, arm(yes, default), arm(no, default)),
        other => return other,
    };
    if yes.is_empty() && !no.is_empty() {
        condition = condition.negated_test();
        std::mem::swap(&mut yes, &mut no);
    }
    // The enclosing condition continues to own evaluation of a nested guard.
    // Conjunction retains the original short circuit.
    if no.is_empty() && matches!(yes.as_slice(), [Statement::If { no, .. }] if no.is_empty()) {
        let Statement::If {
            condition: next,
            yes: nested,
            ..
        } = yes.pop().unwrap()
        else {
            unreachable!()
        };
        condition = Expr::logical(true, vec![condition, next]);
        yes = nested;
    }
    Statement::If { condition, yes, no }
}

fn depends_or_mutates(value: &Expr, local: LocalId) -> bool {
    let mut value = value.clone();
    evaluation::expression_mutates(&mut value)
        || evaluation::effects(&mut value).reads.contains_key(&local)
}
