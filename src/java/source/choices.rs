//! Choose between a value expression and updates to an existing result.
//! Repeated unchanged leaves belong to one initial binding followed by guards;
//! mutations and branch-local captures retain their statement owners.
use super::{Body, Expression, Simple, Statement, access};
use crate::ir::{
    CompareOp, Literal, LocalId, MethodIr, Terminator, ValueId, ValueKind, ValueUse, scheduled,
};
use crate::java::expr::ExprPlanner;
use crate::structure::Predicate;
const MAX_CHOICE_DEPTH: usize = 3;

use std::{
    cell::OnceCell,
    collections::{BTreeMap, BTreeSet},
};

/// One source-construction owner for complete choices in a frozen method.
/// Handler observations include failed analysis, so factoring never retries an
/// expensive whole-method proof for each candidate.
pub(super) struct Planner {
    storage: BTreeSet<LocalId>,
    handler_observers: OnceCell<Option<BTreeSet<LocalId>>>,
}

impl Planner {
    pub(super) fn new(ir: &MethodIr) -> Self {
        Self {
            // A stack merge may remain structured when it has more than two
            // incoming arms. Its temporary still owns an expression, whereas
            // a storage merge owns the surrounding assignment decision tree.
            storage: ir
                .values
                .iter()
                .filter_map(|value| {
                    matches!(value.kind, ValueKind::Phi { slot: Some(_), .. })
                        .then_some(value.local)
                        .flatten()
                })
                .collect(),
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

    pub(super) fn append(&self, body: &mut Body, statement: Statement, expr: &ExprPlanner<'_, '_>) {
        self.append_inner(body, statement, expr, false);
    }

    fn append_inner(
        &self,
        body: &mut Body,
        mut statement: Statement,
        expr: &ExprPlanner<'_, '_>,
        preserve: bool,
    ) {
        let owner = preserve || statement_ladder(expr.ir, &self.storage, &statement);
        if !preserve
            && let Some((destination, baseline, usage)) = default_binding(expr.ir, &statement, self)
        {
            body.push(Statement::Simple(Simple::Assign {
                local: destination,
                value: Expression::Value(usage),
                declaration: false,
            }));
            statement = prune(expr.ir, statement, baseline);
        }
        if let Statement::If { yes, no, .. } = &mut statement {
            for arm in [yes, no] {
                let pending = std::mem::take(arm);
                for child in pending {
                    self.append_inner(arm, child, expr, owner);
                }
            }
        }
        let mut statement = match statement {
            Statement::If {
                condition,
                mut yes,
                mut no,
            } if yes.len() == 1 && no.len() == 1 => match (&yes[0], &no[0]) {
                (Statement::Return(Some(yes_value)), Statement::Return(Some(no_value)))
                    if !owner && fits(expr, &condition, yes_value, no_value) =>
                {
                    let Statement::Return(Some(yes)) = yes.pop().unwrap() else {
                        unreachable!()
                    };
                    let Statement::Return(Some(no)) = no.pop().unwrap() else {
                        unreachable!()
                    };
                    Statement::Return(Some(Expression::select(condition, yes, no)))
                }
                (
                    Statement::Simple(Simple::Assign {
                        local: a,
                        declaration: false,
                        value: yes_value,
                    }),
                    Statement::Simple(Simple::Assign {
                        local: b,
                        declaration: false,
                        value: no_value,
                    }),
                ) if !owner && a == b && fits(expr, &condition, yes_value, no_value) => {
                    let Statement::Simple(Simple::Assign {
                        local, value: yes, ..
                    }) = yes.pop().unwrap()
                    else {
                        unreachable!()
                    };
                    let Statement::Simple(Simple::Assign { value: no, .. }) = no.pop().unwrap()
                    else {
                        unreachable!()
                    };
                    Statement::Simple(Simple::Assign {
                        local,
                        value: Expression::select(condition, yes, no),
                        declaration: false,
                    })
                }
                _ => Statement::If { condition, yes, no },
            },
            statement => statement,
        };
        // Only retained statements use this preference; conditional expressions
        // choose their polarity once, after their operands have been bound.
        if let Statement::If { condition, yes, no } = &mut statement {
            // A conjunction of positive prerequisites is easier to follow than
            // a disjunction of their failures. Do not turn a flat else ladder
            // into a nested first arm merely to change its polarity.
            if !yes.is_empty()
                && !no.is_empty()
                && !matches!(no.as_slice(), [Statement::If { .. }])
                && prefer_inverse(expr.ir, condition)
            {
                let original = std::mem::replace(condition, Expression::Boolean(false));
                *condition = original.negate();
                std::mem::swap(yes, no);
            }
        }
        if let Statement::Return(Some(mut no)) = statement {
            // Inspect the complete adjacent guard sequence before folding any
            // tail. A large lookup keeps all its returns, rather than ending
            // in an arbitrary expression containing only the last few cases.
            let guards: Vec<_> = body.iter().rev().map_while(return_guard).collect();
            let depth = guards
                .iter()
                .filter(|(_, yes)| !logical(expr, yes, &no))
                .count();
            let mut pending = vec![(&no, depth)];
            pending.extend(
                guards
                    .iter()
                    .flat_map(|(condition, yes)| [(*condition, 0), (*yes, depth)]),
            );
            if !owner
                && depth <= MAX_CHOICE_DEPTH
                && fits_values(expr, pending)
                && guards
                    .iter()
                    .all(|(condition, yes)| fits(expr, condition, yes, &no))
            {
                let count = guards.len();
                for _ in 0..count {
                    let Statement::If {
                        condition, mut yes, ..
                    } = body.pop().unwrap()
                    else {
                        unreachable!()
                    };
                    let Statement::Return(Some(yes)) = yes.pop().unwrap() else {
                        unreachable!()
                    };
                    no = Expression::select(condition, yes, no);
                }
            }
            body.push(Statement::Return(Some(no)));
        } else {
            body.push(statement);
        }
    }
}

fn prefer_inverse(ir: &MethodIr, expression: &Expression) -> bool {
    let Expression::Predicate { predicate, negated } = expression else {
        return false;
    };
    if !matches!(
        (predicate, negated),
        (Predicate::Or(_), false) | (Predicate::And(_), true)
    ) {
        return false;
    }
    let mut negative = [0, 0];
    let mut pending = vec![predicate];
    while let Some(predicate) = pending.pop() {
        match predicate {
            Predicate::Atom {
                head,
                negated: atom,
            } => {
                let Terminator::Branch { condition, .. } = &ir.blocks[*head].terminator else {
                    return false;
                };
                for (inverse, count) in negative.iter_mut().enumerate() {
                    let op = if *negated ^ *atom ^ (inverse != 0) {
                        condition.op.inverted()
                    } else {
                        condition.op
                    };
                    let boolean_zero = [
                        (condition.left, condition.right),
                        (condition.right, condition.left),
                    ]
                    .into_iter()
                    .any(|(value, zero)| {
                        ir.values[value].ty == crate::classfile::JvmType::Boolean
                            && matches!(ir.values[zero].kind, ValueKind::Literal(Literal::Int(0)))
                    });
                    *count += usize::from(if boolean_zero {
                        op == CompareOp::Eq
                    } else {
                        op == CompareOp::Ne
                    });
                }
            }
            Predicate::And(terms) | Predicate::Or(terms) => pending.extend(terms),
            Predicate::Select { .. } => return false,
        }
    }
    negative[1] < negative[0]
}

fn return_guard(statement: &Statement) -> Option<(&Expression, &Expression)> {
    if let Statement::If { condition, yes, no } = statement
        && no.is_empty()
        && let [Statement::Return(Some(value))] = yes.as_slice()
    {
        Some((condition, value))
    } else {
        None
    }
}

/// A complete decision tree has one presentation owner. Storage decisions
/// retain a ladder; stack results and returns permit bounded value expressions.
/// A single storage diamond includes ordinary read-modify-write computations.
fn statement_ladder(ir: &MethodIr, storage: &BTreeSet<LocalId>, statement: &Statement) -> bool {
    let mut pending = vec![statement];
    let mut destination = None;
    let mut decisions = 0;
    while let Some(statement) = pending.pop() {
        let target = match statement {
            Statement::If { yes, no, .. } if yes.len() == 1 && no.len() == 1 => {
                decisions += 1;
                pending.extend([&yes[0], &no[0]]);
                continue;
            }
            Statement::Simple(Simple::Assign { local, .. }) => Some(*local),
            Statement::Return(Some(_)) => None,
            _ => return false,
        };
        if destination.is_some_and(|previous| previous != target) {
            return false;
        }
        destination = Some(target);
    }
    let Some(destination) = destination else {
        return false;
    };
    let ty = destination.map_or(&ir.return_type, |local| &ir.locals[local].ty);
    if *ty == crate::classfile::JvmType::Boolean {
        return false;
    }
    let limit = if destination.is_some_and(|local| storage.contains(&local)) {
        1
    } else {
        MAX_CHOICE_DEPTH
    };
    decisions > limit
}

fn logical(expr: &ExprPlanner<'_, '_>, yes: &Expression, no: &Expression) -> bool {
    matches!(yes, Expression::Boolean(_)) && no.canonical(expr)
        || matches!(no, Expression::Boolean(_)) && yes.canonical(expr)
}

fn fits(
    expr: &ExprPlanner<'_, '_>,
    condition: &Expression,
    yes: &Expression,
    no: &Expression,
) -> bool {
    if !logical(expr, yes, no)
        && [condition, yes, no]
            .into_iter()
            .any(|value| mutates(expr.ir, value))
    {
        return false;
    }
    // Keep the existing bound on new source trees. Count logical reductions
    // separately: a chain returning Boolean constants becomes &&/||, not a
    // ladder of value alternatives.
    let choices = usize::from(!logical(expr, yes, no));
    fits_values(expr, vec![(condition, 0), (yes, choices), (no, choices)])
}

fn fits_values(expr: &ExprPlanner<'_, '_>, mut pending: Vec<(&Expression, usize)>) -> bool {
    let mut budget = 48usize;
    while let Some((value, choices)) = pending.pop() {
        let Some(left) = budget.checked_sub(1) else {
            return false;
        };
        budget = left;
        if let Expression::Select { condition, yes, no } = value {
            let choices = choices + usize::from(!logical(expr, yes, no));
            if choices > MAX_CHOICE_DEPTH {
                return false;
            }
            pending.extend([
                (condition.as_ref(), choices),
                (yes.as_ref(), choices),
                (no.as_ref(), choices),
            ]);
        }
    }
    true
}

fn mutates(ir: &MethodIr, expression: &Expression) -> bool {
    let mut result = false;
    let mut updates = false;
    access::for_each_value(
        ir,
        expression,
        |value| {
            result |= scheduled::values(ir, value).operations().any(|value| {
                let kind = &ir.values[value].kind;
                kind.local_write().is_some()
                    || matches!(
                        kind,
                        ValueKind::Update { .. } | ValueKind::CompoundUpdate { .. }
                    )
            });
        },
        |_| updates = true,
    );
    result || updates
}

/// An unchanged leaf is a stable binding or the same literal definition.
/// Its assignment context supplies conversions, including narrow-to-int copies.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Baseline {
    Local(LocalId),
    Literal(ValueId),
}

fn baseline(ir: &MethodIr, expression: &Expression) -> Option<Baseline> {
    let Expression::Value(usage) = expression else {
        return None;
    };
    usage.local(ir).map(Baseline::Local).or_else(|| {
        matches!(ir.values[usage.value()].kind, ValueKind::Literal(_))
            .then_some(Baseline::Literal(usage.value()))
    })
}

fn default_binding(
    ir: &MethodIr,
    statement: &Statement,
    planner: &Planner,
) -> Option<(LocalId, Baseline, ValueUse)> {
    if !matches!(statement, Statement::If { .. }) {
        return None;
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
            return None;
        }
        let Some((statement, rest)) = body.split_first() else {
            if !initialized {
                return None;
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
                local,
                value,
                declaration: false,
            }) => {
                if destination.is_some_and(|previous| previous != *local) {
                    return None;
                }
                destination = Some(*local);
                if !rest.is_empty() {
                    initializers.push(value);
                }
                leaves.push(value);
                pending.push((rest, depth + 1, true));
            }
            _ => return None,
        }
    }
    let destination = destination?;
    let mut baselines = BTreeMap::<Baseline, (usize, ValueUse)>::new();
    for value in &leaves {
        if let Expression::Value(usage) = value
            && let Some(source) = baseline(ir, value)
            && source != Baseline::Local(destination)
        {
            baselines.entry(source).or_insert((0, *usage)).0 += 1;
        }
    }
    let (&default, &(_, usage)) = baselines.iter().find(|(_, (count, _))| *count >= 2)?;
    // An arbitrary earlier assignment followed by a reset to baseline must
    // keep both writes in order. Only a baseline initializer can move out.
    if initializers
        .iter()
        .any(|value| baseline(ir, value) != Some(default))
        || leaves
            .iter()
            .all(|value| baseline(ir, value) == Some(default))
        || leaves.iter().any(|value| {
            let accesses = access::expression(ir, value);
            accesses.contains(&destination) || mutates(ir, value)
        })
        || conditions
            .iter()
            .any(|value| access::expression(ir, value).contains(&destination) || mutates(ir, value))
    {
        return None;
    }
    // Initializing the result earlier must not change a catch's view if a
    // condition or an arm throws before its original result assignment.
    if planner
        .handler_observers(ir)
        .is_none_or(|locals| locals.contains(&destination))
    {
        return None;
    }
    Some((destination, default, usage))
}

fn prune(ir: &MethodIr, statement: Statement, default: Baseline) -> Statement {
    fn arm(ir: &MethodIr, body: Body, default: Baseline) -> Body {
        body.into_iter().filter_map(|statement| {
            (!matches!(&statement, Statement::Simple(Simple::Assign { value, .. }) if baseline(ir, value) == Some(default)))
                .then(|| prune(ir, statement, default))
        }).collect()
    }
    let (mut condition, mut yes, mut no) = match statement {
        Statement::If { condition, yes, no } => {
            (condition, arm(ir, yes, default), arm(ir, no, default))
        }
        other => return other,
    };
    if yes.is_empty() && !no.is_empty() {
        condition = condition.negate();
        std::mem::swap(&mut yes, &mut no);
    }
    // The enclosing condition continues to own evaluation of a nested guard.
    // Binding this logical choice emits && with the original short circuit.
    if no.is_empty() && matches!(yes.as_slice(), [Statement::If { no, .. }] if no.is_empty()) {
        let Statement::If {
            condition: next,
            yes: nested,
            ..
        } = yes.pop().unwrap()
        else {
            unreachable!()
        };
        condition = Expression::select(condition, next, Expression::Boolean(false));
        yes = nested;
    }
    Statement::If { condition, yes, no }
}
