//! Final is a source property as well as a classfile flag. Check the selected
//! Java control flow, including dispatch backedges, before emitting declarations.
//! Relaxing a modifier never moves or suppresses an original field publication.
use super::{Body, Expression, LoopHeader, Simple, Statement};
use crate::{ir, java::expr::ExprPlanner, program::MemberId};
use std::collections::{BTreeMap, BTreeSet, HashSet};

const UNASSIGNED: u8 = 1;
const ASSIGNED: u8 = 2;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Exit {
    Return,
    Throw,
    Break(Option<String>),
    Continue(Option<String>),
}

#[derive(Default)]
struct Flow {
    normal: u8,
    abrupt: BTreeMap<Exit, u8>,
}

impl Flow {
    fn entry(normal: u8) -> Self {
        Self {
            normal,
            ..Self::default()
        }
    }

    fn merge(&mut self, other: Self) {
        self.normal |= other.normal;
        for (exit, state) in other.abrupt {
            *self.abrupt.entry(exit).or_default() |= state;
        }
    }

    fn take(&mut self, exit: Exit) -> u8 {
        self.abrupt.remove(&exit).unwrap_or(0)
    }
}

pub(super) fn check(
    expr: &ExprPlanner<'_, '_>,
    body: &Body,
    delegates: bool,
    initializes_fields: bool,
) {
    let program = expr.cx.program;
    let owner = expr.cx.types.owner;
    if program.class(owner).access_flags & 0x0200 != 0 {
        // Interface publications are reconstructed as declaration initializers.
        return;
    }
    let mut fields = BTreeSet::new();
    visit(body, &mut |simple| {
        if let Some((field, _)) = written(expr, simple) {
            fields.insert(field);
        }
    });
    for field in fields {
        if !expr.cx.retains_final(field) {
            continue;
        }
        let declaration = program.field(field);
        let is_static = declaration.access_flags & 0x0008 != 0;
        let initializer = if is_static { "<clinit>" } else { "<init>" };
        let legal_context = initializes_fields
            && expr.member.is_some_and(|member| {
                member.class == field.class && program.method(member).name == initializer
            });
        let initialized =
            is_static && declaration.constant_value.is_some() || !is_static && delegates;
        let mut check = Check {
            expr,
            field,
            invalid: !legal_context,
        };
        let flow = check.body(body, if initialized { ASSIGNED } else { UNASSIGNED });
        let exits = flow.normal | flow.abrupt.get(&Exit::Return).copied().unwrap_or(0);
        if check.invalid || exits & UNASSIGNED != 0 {
            expr.cx.relax_final(field);
        }
    }
}

fn written(expr: &ExprPlanner<'_, '_>, simple: &Simple) -> Option<(MemberId, Option<ir::ValueId>)> {
    let Simple::Scheduled(ir::Statement::PutField {
        field, receiver, ..
    }) = simple
    else {
        return None;
    };
    let id = expr
        .cx
        .program
        .resolve_field(&field.owner, &field.name, &field.descriptor)?;
    (id.class == expr.cx.types.owner).then_some((id, *receiver))
}

fn visit(body: &Body, action: &mut impl FnMut(&Simple)) {
    for statement in body {
        match statement {
            Statement::Simple(simple) => action(simple),
            Statement::If { yes, no, .. } => {
                visit(yes, action);
                visit(no, action);
            }
            Statement::Loop { header, body, .. } => {
                if let LoopHeader::For {
                    initializer,
                    update,
                    ..
                } = header
                {
                    if let Some(initializer) = initializer {
                        action(initializer);
                    }
                    action(update);
                }
                visit(body, action);
            }
            Statement::Switch { cases, .. } => {
                for case in cases {
                    visit(&case.body, action);
                }
            }
            Statement::Block { body, .. } | Statement::Synchronized { body, .. } => {
                visit(body, action)
            }
            Statement::Try {
                body,
                catches,
                cleanup,
            } => {
                visit(body, action);
                for catch in catches {
                    visit(&catch.body, action);
                }
                if let Some(cleanup) = cleanup {
                    visit(cleanup, action);
                }
            }
            _ => (),
        }
    }
}

struct Check<'a, 'b, 'c> {
    expr: &'c ExprPlanner<'a, 'b>,
    field: MemberId,
    invalid: bool,
}

impl Check<'_, '_, '_> {
    fn simple(&mut self, simple: &Simple, state: u8) -> u8 {
        let Some((field, receiver)) = written(self.expr, simple) else {
            return state;
        };
        if field != self.field || state == 0 {
            return state;
        }
        if state & ASSIGNED != 0
            || receiver.is_some_and(|receiver| {
                !crate::java::constructors::is_this(self.expr.ir, receiver, &mut HashSet::new())
            })
        {
            self.invalid = true;
        }
        ASSIGNED
    }

    fn body(&mut self, body: &Body, state: u8) -> Flow {
        let mut flow = Flow::entry(state);
        for statement in body {
            let next = self.statement(statement, flow.normal);
            flow.normal = 0;
            flow.merge(next);
        }
        flow
    }

    fn statement(&mut self, statement: &Statement, state: u8) -> Flow {
        let mut flow = Flow::default();
        match statement {
            Statement::Simple(simple) => flow.normal = self.simple(simple, state),
            Statement::Return(_) => {
                flow.abrupt.insert(Exit::Return, state);
            }
            Statement::Throw(_) => {
                flow.abrupt.insert(Exit::Throw, state);
            }
            Statement::Break(label) => {
                flow.abrupt.insert(Exit::Break(label.clone()), state);
            }
            Statement::Continue(label) => {
                flow.abrupt.insert(Exit::Continue(label.clone()), state);
            }
            Statement::If { yes, no, .. } => {
                flow = self.body(yes, state);
                flow.merge(self.body(no, state));
            }
            Statement::Block { label, body } => {
                flow = self.body(body, state);
                if label.is_some() {
                    flow.normal |= flow.take(Exit::Break(label.clone()));
                }
            }
            Statement::Synchronized { body, .. } => flow = self.body(body, state),
            Statement::Switch { label, cases, .. } => {
                if !cases.iter().any(|case| case.default) {
                    flow.normal = state;
                }
                let mut fallthrough = 0;
                for case in cases {
                    let mut branch = self.body(&case.body, state | fallthrough);
                    fallthrough = branch.normal;
                    branch.normal = 0;
                    flow.merge(branch);
                }
                flow.normal |= fallthrough | flow.take(Exit::Break(None));
                if label.is_some() {
                    flow.normal |= flow.take(Exit::Break(label.clone()));
                }
            }
            Statement::Loop {
                label,
                header,
                body,
            } => {
                let (condition, initial) = match header {
                    LoopHeader::For {
                        initializer,
                        condition,
                        ..
                    } => (
                        condition,
                        initializer
                            .as_ref()
                            .map_or(state, |simple| self.simple(simple, state)),
                    ),
                    LoopHeader::While(condition) | LoopHeader::DoWhile(condition) => {
                        (condition, state)
                    }
                };
                let repeats = !matches!(condition, Expression::Boolean(false));
                let mut entry = initial;
                loop {
                    let mut iteration = self.body(body, entry);
                    let mut back = iteration.normal | iteration.take(Exit::Continue(None));
                    if label.is_some() {
                        back |= iteration.take(Exit::Continue(label.clone()));
                    }
                    if let LoopHeader::For { update, .. } = header {
                        back = self.simple(update, back);
                    }
                    let exit = iteration.take(Exit::Break(None))
                        | if label.is_some() {
                            iteration.take(Exit::Break(label.clone()))
                        } else {
                            0
                        };
                    iteration.normal = exit;
                    if !matches!(condition, Expression::Boolean(true)) {
                        iteration.normal |= back;
                        if !matches!(header, LoopHeader::DoWhile(_)) {
                            iteration.normal |= initial;
                        }
                    }
                    flow.merge(iteration);
                    let next = entry | if repeats { back } else { 0 };
                    if next == entry {
                        break;
                    }
                    entry = next;
                }
            }
            Statement::Try {
                body,
                catches,
                cleanup,
            } => {
                flow = self.body(body, state);
                // Java requires DU before every possible protected assignment
                // for a catch/finally assignment, even for precise JVM handlers.
                let mut exceptional = state;
                visit(body, &mut |simple| {
                    if written(self.expr, simple).is_some_and(|(field, _)| field == self.field) {
                        exceptional |= ASSIGNED;
                    }
                });
                for catch in catches {
                    flow.merge(self.body(&catch.body, exceptional));
                }
                if let Some(cleanup) = cleanup {
                    let incoming = std::mem::take(&mut flow);
                    flow = self.body(cleanup, incoming.normal);
                    for (exit, state) in incoming.abrupt {
                        let mut cleaned = self.body(cleanup, state);
                        *cleaned.abrupt.entry(exit).or_default() |= cleaned.normal;
                        cleaned.normal = 0;
                        flow.merge(cleaned);
                    }
                    // An implicit exception can also run the cleanup.
                    let _ = self.body(cleanup, exceptional);
                }
            }
        }
        flow
    }
}
