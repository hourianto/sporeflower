use super::{
    Body, Case, Catch, Expression, LoopHeader, Selector, Simple, Statement as Source, Throw,
    access, append, facts::Facts, loops, switches,
};
use crate::java::expr::ExprPlanner;
use crate::{
    Result,
    ir::{BlockId, LocalId, Statement, Terminator, ValueUse, local_effects},
    structure::{DispatchCase, FlowEnd, RegionId, Stmt, StructuredMethod, tree::Binder},
};
use anyhow::{bail, ensure};
use std::collections::{BTreeSet, HashSet};

pub(in crate::java) struct Options<'a> {
    pub initialize: bool,
    pub selected: Option<&'a BTreeSet<LocalId>>,
    pub skip_constructor: Option<(BlockId, usize)>,
    pub delegates: bool,
    pub initializes_fields: bool,
    pub initializer: bool,
    /// Falling out of this source member has the same effect as `return;`.
    pub returns_void: bool,
}

pub(in crate::java) fn lower(
    expr: &ExprPlanner<'_, '_>,
    tree: &StructuredMethod,
    body: &[Stmt],
    options: Options<'_>,
) -> Result<Body> {
    let caught = crate::java::scopes::lexical_catches(expr.ir, &tree.body);
    let mut lowerer = Lowerer {
        expr,
        caught,
        generated_names: None,
        labels: collect_labels(&tree.body),
        active: Vec::new(),
        control_accesses: loops::control_accesses(expr.ir),
        facts: Facts::default(),
        return_observers: BTreeSet::new(),
        suppressed_monitors: tree.suppressed_monitors.iter().copied().collect(),
        options,
    };
    let mut output = lowerer.body(body, None, lowerer.options.returns_void)?;
    crate::java::source_locals::place(
        expr.ir,
        &mut output,
        &lowerer.caught,
        lowerer.options.selected,
        lowerer.options.initialize,
    );
    Ok(output)
}

struct Lowerer<'a, 'b, 'c> {
    expr: &'c ExprPlanner<'a, 'b>,
    caught: HashSet<LocalId>,
    generated_names: Option<crate::java::local_names::Allocator<'a>>,
    labels: HashSet<RegionId>,
    active: Vec<Binder>,
    control_accesses: crate::index::IdVec<LocalId, usize>,
    facts: Facts,
    return_observers: BTreeSet<LocalId>,
    suppressed_monitors: HashSet<(BlockId, usize)>,
    options: Options<'c>,
}

impl Lowerer<'_, '_, '_> {
    fn generated_name(&mut self, stem: &str) -> String {
        // Ordinary bodies need no synthetic bindings. Reserve the method's
        // locals and the class's fields only when a generated catch needs them.
        self.generated_names
            .get_or_insert_with(|| {
                let mut allocator = crate::java::local_names::Allocator::generated(
                    self.expr.ir,
                    self.expr.cx.program,
                    self.expr.cx.types,
                );
                for local in self.expr.ir.locals.ids() {
                    allocator.reserve(self.expr.local_name(local));
                }
                allocator
            })
            .allocate(stem)
    }

    fn body(
        &mut self,
        input: &[Stmt],
        omitted: Option<loops::Site>,
        returns_void: bool,
    ) -> Result<Body> {
        // A trailing bare return also supplies this continuation to the
        // preceding statement. Ignore empty scheduled fragments, but never
        // move a value return or an effect through a lexical boundary.
        let mut void_tail = returns_void;
        let mut tail = input.len();
        while let Some(statement) = tail.checked_sub(1).map(|index| &input[index]) {
            if statement
                .scheduled(self.expr.ir)
                .is_some_and(|scheduled| scheduled.statements.is_empty())
            {
                tail -= 1;
            } else if matches!(statement, Stmt::Return(block)
                if matches!(self.expr.ir.blocks[*block].terminator, Terminator::Return(None)))
            {
                void_tail = true;
                tail -= 1;
            } else {
                break;
            }
        }
        let mut body = Vec::new();
        for (index, statement) in input.iter().enumerate() {
            if let Stmt::Loop {
                region,
                header,
                body: nested,
                continuation,
            } = statement
            {
                // Backedges can change a local before any later iteration.
                let outer = self
                    .facts
                    .clone()
                    .without_writes(self.expr, std::slice::from_ref(statement));
                self.facts = outer.clone();
                let parts = loops::Parts::recognize(
                    self.expr,
                    *region,
                    *header,
                    nested,
                    *continuation,
                    index.checked_sub(1).map(|index| &input[index]),
                    omitted,
                )?;
                if parts.initializer.is_some() {
                    // The initializer is the immediately preceding scheduled
                    // assignment. Transfer its one source occurrence to the header.
                    ensure!(
                        matches!(body.pop(), Some(Source::Simple(_))),
                        "loop initializer lost its source assignment"
                    );
                }
                self.active.push(Binder::Loop {
                    region: *region,
                    header: *header,
                });
                let nested = self.body(&nested[parts.body_start..], parts.update, false)?;
                self.active.pop();
                self.facts = outer;
                body.extend(loops::construct(self.label(*region), parts.header, nested));
            } else if let Some(scheduled) = statement.scheduled(self.expr.ir) {
                for (index, statement) in scheduled.iter() {
                    self.facts.statement(self.expr, statement, true);
                    let site = (scheduled.block, index);
                    if Some(site) == omitted
                        || Some(site) == self.options.skip_constructor
                        || self.suppressed_monitors.contains(&site)
                    {
                        continue;
                    }
                    if let Some(statement) = self.scheduled(statement, site)? {
                        append(&mut body, statement, self.expr);
                    }
                }
            } else {
                for mut statement in
                    self.statement(statement, void_tail && index >= tail.saturating_sub(1))?
                {
                    if let Source::Return(Some(Expression::Value(ValueUse::Reference(value)))) =
                        &statement
                        && let Some(local) = self.expr.ir.values[*value].local
                        && !self.return_observers.contains(&local)
                        && matches!(body.last(), Some(Source::Simple(Simple::Assign { local: assigned, .. })) if *assigned == local)
                        && return_binding(self.expr, local, body.last().unwrap())
                    {
                        let Source::Simple(Simple::Assign { value, .. }) = body.pop().unwrap()
                        else {
                            unreachable!()
                        };
                        statement = Source::Return(Some(value));
                    }
                    append(&mut body, statement, self.expr);
                }
            }
            // A proven branch can expose its terminal transfer at this scope.
            // Its old continuation is unreachable and Java must not print it.
            if !super::completion::can_complete(&body) {
                break;
            }
        }
        if returns_void && matches!(body.last(), Some(Source::Return(None))) {
            body.pop();
        }
        loops::expand_finite(self.expr.ir, &mut body, &self.control_accesses);
        Ok(body)
    }

    fn label(&self, region: RegionId) -> Option<String> {
        self.labels
            .contains(&region)
            .then(|| format!("label{region}"))
    }

    fn bare_void_return(&self, body: &[Stmt]) -> bool {
        body.iter().all(|statement| {
            matches!(statement, Stmt::Return(block)
                if matches!(self.expr.ir.blocks[*block].terminator, Terminator::Return(None)))
                || statement
                    .scheduled(self.expr.ir)
                    .is_some_and(|scheduled| scheduled.statements.is_empty())
        })
    }

    fn statement(&mut self, statement: &Stmt, returns_void: bool) -> Result<Body> {
        Ok(vec![match statement {
            Stmt::Return(block) | Stmt::Throw(block) => self.terminal(*block)?,
            Stmt::Unreachable(block) => bail!("unreachable bytecode terminator in block {block}"),
            Stmt::If {
                condition,
                then_body,
                else_body,
            } => {
                if let Some(truth) = self.facts.known(self.expr, condition) {
                    return self.body(
                        if truth { then_body } else { else_body },
                        None,
                        returns_void,
                    );
                }
                self.facts.predicate(self.expr, condition);
                let incoming = self.facts.clone();
                self.facts.assume(self.expr, condition, true);
                // Keep a guard's sole return rather than leave an empty if.
                // Its condition still has to be evaluated at this point.
                let yes = self.body(
                    then_body,
                    None,
                    returns_void && !self.bare_void_return(then_body),
                )?;
                let yes_facts = self.facts.clone();
                self.facts = incoming;
                self.facts.assume(self.expr, condition, false);
                let no = self.body(
                    else_body,
                    None,
                    returns_void && !self.bare_void_return(else_body),
                )?;
                self.facts = match (
                    crate::structure::can_complete(then_body),
                    crate::structure::can_complete(else_body),
                ) {
                    (true, true) => yes_facts.join(&self.facts),
                    (true, false) => yes_facts,
                    (false, true) => self.facts.clone(),
                    (false, false) => Facts::default(),
                };
                Source::If {
                    condition: Expression::predicate(condition, false),
                    yes,
                    no,
                }
            }
            Stmt::Switch {
                region,
                head,
                cases,
            } => {
                let Terminator::Switch { value, .. } = &self.expr.ir.blocks[*head].terminator
                else {
                    bail!("switch head is not a switch")
                };
                let value = Selector::Value(*value);
                let incoming = self
                    .facts
                    .clone()
                    .without_writes(self.expr, std::slice::from_ref(statement));
                self.active.push(Binder::Switch(*region));
                let mut cases = cases
                    .iter()
                    .map(|case| {
                        self.facts = incoming.clone();
                        Ok(Case {
                            keys: case.keys.clone(),
                            default: case.default,
                            scoped: false,
                            body: self.body(&case.body, None, false)?,
                        })
                    })
                    .collect::<Result<_>>()?;
                self.active.pop();
                self.facts = incoming;
                switches::group(self.expr.ir, &mut cases);
                Source::Switch {
                    label: self.label(*region),
                    value,
                    cases,
                }
            }
            Stmt::Label { region, body } => {
                let outgoing = self.facts.clone().without_writes(self.expr, body);
                let body = self.body(body, None, returns_void)?;
                self.facts = outgoing;
                Source::Block {
                    label: Some(format!("label{region}")),
                    body,
                }
            }
            Stmt::Break { target } => Source::Break(
                break_label(&self.active, *target).map(|target| format!("label{target}")),
            ),
            Stmt::Continue { header } => Source::Continue(
                continue_label(&self.active, *header)?.map(|target| format!("label{target}")),
            ),
            Stmt::Try { body, catches } => {
                if catches.is_empty() {
                    return self.body(body, None, returns_void);
                }
                self.facts = Facts::default();
                let mut source_body = Vec::new();
                let observers = self.return_observers.clone();
                let mut source_catches = Vec::new();
                for catch in catches {
                    self.facts = Facts::default();
                    let ty = catch.catch_type.as_deref().unwrap_or("java/lang/Throwable");
                    let local = self.catch_local(catch.handler);
                    let lexical = local.filter(|local| self.caught.contains(local));
                    let name = match lexical {
                        Some(local) => self.expr.local_name(local).to_owned(),
                        None => self.generated_name("e"),
                    };
                    let mut body = Vec::new();
                    if let Some(local) = local.filter(|_| lexical.is_none()) {
                        body.push(Source::Simple(Simple::CatchBind {
                            local,
                            name: name.clone(),
                            ty: None,
                        }));
                    }
                    body.extend(self.body(&catch.body, None, returns_void)?);
                    source_catches.push(Catch {
                        catch_type: ty.into(),
                        ty: self.expr.cx.types.class(ty),
                        name,
                        body,
                    });
                }
                for catch in &source_catches {
                    self.return_observers
                        .extend(access::body(self.expr.ir, &catch.body));
                }
                self.facts = Facts::default();
                source_body.extend(self.body(body, None, returns_void)?);
                self.return_observers = observers;
                self.facts = Facts::default();
                Source::Try {
                    body: source_body,
                    catches: source_catches,
                    cleanup: None,
                }
            }
            Stmt::TryFinally { body, cleanup, .. } => {
                self.facts = Facts::default();
                // A return in finally overrides the pending completion. Its
                // own fallthrough must never be treated as a method return.
                let cleanup = self.body(cleanup, None, false)?;
                let observers = self.return_observers.clone();
                self.return_observers
                    .extend(access::body(self.expr.ir, &cleanup));
                self.facts = Facts::default();
                let body = self.body(body, None, returns_void)?;
                self.return_observers = observers;
                self.facts = Facts::default();
                return Ok(super::completion::with_cleanup(body, cleanup));
            }
            Stmt::Synchronized { monitor, body } => {
                self.facts.forget_heap();
                self.facts.invalidate(local_effects::expression(
                    self.expr.ir,
                    ValueUse::Reference(*monitor),
                ));
                let body = self.body(body, None, returns_void)?;
                self.facts.forget_heap();
                Source::Synchronized {
                    monitor: Expression::Value(ValueUse::Reference(*monitor)),
                    body,
                }
            }
            Stmt::Dispatch {
                region,
                entry,
                cases,
                exits,
            } => {
                self.facts = Facts::default();
                let observers = std::mem::replace(
                    &mut self.return_observers,
                    self.expr.ir.locals.ids().collect(),
                );
                let result = self.dispatch(*region, *entry, cases, exits);
                self.return_observers = observers;
                self.facts = Facts::default();
                return result;
            }
            Stmt::DispatchContinue { region, target } => {
                return Ok(vec![
                    Source::Simple(Simple::Dispatch {
                        state: format!("$state{region}"),
                        target: *target,
                        declare: false,
                    }),
                    Source::Continue(Some(format!("$dispatch{region}"))),
                ]);
            }
            Stmt::Jump(_) | Stmt::BoundRegion { .. } => {
                bail!("unbound region exit reached Java source lowering")
            }
            Stmt::Basic(_) | Stmt::Statements { .. } | Stmt::Loop { .. } => {
                unreachable!("handled by lexical source lowering")
            }
        }])
    }

    fn scheduled(&self, statement: &Statement, site: loops::Site) -> Result<Option<Source>> {
        let simple = match statement {
            Statement::Assign { local, value } => {
                ensure!(
                    !self.expr.ir.locals[*local].is_this,
                    "attempted assignment to this"
                );
                Simple::Assign {
                    local: *local,
                    value: Expression::value(self.expr.ir, *value, &self.expr.ir.locals[*local].ty),
                    declaration: false,
                }
            }
            Statement::PutField {
                field,
                receiver,
                value,
                ..
            } if self.options.initializer
                && crate::java::fields::redundant_constant_store(
                    self.expr.cx.program,
                    self.expr.cx.types.owner,
                    self.expr.ir,
                    field,
                    *receiver,
                    *value,
                ) =>
            {
                return Ok(None);
            }
            Statement::Monitor { .. } => {
                bail!("unstructured monitor operation in block {}", site.0)
            }
            statement => Simple::Scheduled(statement.clone()),
        };
        Ok(Some(Source::Simple(simple)))
    }

    fn terminal(&self, block: BlockId) -> Result<Source> {
        match &self.expr.ir.blocks[block].terminator {
            Terminator::Return(value) => {
                if value.is_none() && self.options.initializer {
                    return Ok(Source::Break(Some("$initializer".into())));
                }
                Ok(Source::Return(value.map(|value| {
                    Expression::value(
                        self.expr.ir,
                        ValueUse::Reference(value),
                        &self.expr.ir.return_type,
                    )
                })))
            }
            Terminator::Throw { value, pc } => Ok(Source::Throw(Throw::Value {
                value: *value,
                pc: *pc,
            })),
            _ => bail!("structured terminal points to a nonterminal block {block}"),
        }
    }

    fn catch_local(&self, handler: BlockId) -> Option<LocalId> {
        self.expr.ir.blocks[handler]
            .caught_exception
            .and_then(|value| self.expr.ir.values[value].local)
    }

    fn dispatch(
        &mut self,
        region: usize,
        entry: BlockId,
        input: &[DispatchCase],
        exits: &[(BlockId, RegionId)],
    ) -> Result<Body> {
        let state = format!("$state{region}");
        let blocks: HashSet<_> = input.iter().map(|case| case.head).collect();
        self.active.push(Binder::Dispatch(region));
        let mut cases = Vec::new();
        for case in input {
            self.facts = Facts::default();
            let block = case.head;
            let mut body = Vec::new();
            body.extend(self.body(&case.body, None, false)?);
            match &case.end {
                FlowEnd::Goto(target) => {
                    body.extend(dispatch_edge(*target, region, &state, &blocks, exits)?)
                }
                FlowEnd::Branch {
                    then_target,
                    else_target,
                } => {
                    ensure!(
                        matches!(
                            self.expr.ir.blocks[block].terminator,
                            Terminator::Branch { .. }
                        ),
                        "dispatch branch head is not a branch"
                    );
                    body.push(Source::If {
                        condition: Expression::predicate(
                            &crate::structure::Predicate::Atom {
                                head: block,
                                negated: false,
                            },
                            false,
                        ),
                        yes: dispatch_edge(*then_target, region, &state, &blocks, exits)?,
                        no: dispatch_edge(*else_target, region, &state, &blocks, exits)?,
                    });
                }
                FlowEnd::Switch { cases, default } => {
                    let Terminator::Switch { value, .. } = &self.expr.ir.blocks[block].terminator
                    else {
                        bail!("dispatch switch head is not a switch")
                    };
                    let mut source_cases = cases
                        .iter()
                        .map(|(key, target)| {
                            Ok(Case {
                                keys: vec![*key],
                                default: false,
                                scoped: false,
                                body: dispatch_edge(*target, region, &state, &blocks, exits)?,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    source_cases.push(Case {
                        keys: Vec::new(),
                        default: true,
                        scoped: false,
                        body: dispatch_edge(*default, region, &state, &blocks, exits)?,
                    });
                    body.push(Source::Switch {
                        label: None,
                        value: Selector::Value(*value),
                        cases: source_cases,
                    });
                }
                FlowEnd::Return | FlowEnd::Throw => body.push(self.terminal(block)?),
                FlowEnd::Unreachable => ensure!(
                    !crate::structure::can_complete(&case.body),
                    "dispatch block {block} falls into an unreachable terminator"
                ),
                FlowEnd::Multi(_) => ensure!(
                    !crate::structure::can_complete(&case.body),
                    "multi-exit region has an unbound fallthrough"
                ),
            }
            if !case.handlers.is_empty() {
                let name = self.generated_name("e");
                let mut catches = Vec::new();
                for handler in &case.handlers {
                    let ty = self.expr.cx.types.class(
                        handler
                            .catch_type
                            .as_deref()
                            .unwrap_or("java/lang/Throwable"),
                    );
                    let mut handler_body = Vec::new();
                    if let Some(local) = self.catch_local(handler.target) {
                        handler_body.push(Source::Simple(Simple::CatchBind {
                            local,
                            name: name.clone(),
                            ty: Some(ty.clone()),
                        }));
                    }
                    handler_body.extend(dispatch_edge(
                        handler.target,
                        region,
                        &state,
                        &blocks,
                        exits,
                    )?);
                    catches.push(Catch {
                        catch_type: handler
                            .catch_type
                            .clone()
                            .unwrap_or_else(|| "java/lang/Throwable".into()),
                        ty,
                        name: name.clone(),
                        body: handler_body,
                    });
                }
                body = vec![Source::Try {
                    body,
                    catches,
                    cleanup: None,
                }];
            }
            cases.push(Case {
                keys: vec![block.index() as i32],
                default: false,
                scoped: true,
                body,
            });
        }
        cases.push(Case {
            keys: Vec::new(),
            default: true,
            scoped: false,
            body: vec![Source::Throw(Throw::InvalidDispatch)],
        });
        self.active.pop();
        Ok(vec![
            Source::Simple(Simple::Dispatch {
                state: state.clone(),
                target: entry,
                declare: true,
            }),
            Source::Loop {
                label: Some(format!("$dispatch{region}")),
                header: LoopHeader::While(Expression::Boolean(true)),
                body: vec![Source::Switch {
                    label: None,
                    value: Selector::Dispatch(state),
                    cases,
                }],
            },
        ])
    }
}

fn return_binding(expr: &ExprPlanner<'_, '_>, local: LocalId, statement: &Source) -> bool {
    let ty = &expr.ir.locals[local].ty;
    if *ty == expr.ir.return_type {
        return true;
    }
    // Removing a binding must not remove its narrowing or checked cast. A
    // reference producer already of the binding type needs no such conversion.
    let Source::Simple(Simple::Assign {
        value: Expression::Value(usage),
        ..
    }) = statement
    else {
        return false;
    };
    let actual = usage
        .local(expr.ir)
        .map_or(&expr.ir.values[usage.value()].ty, |local| {
            &expr.ir.locals[local].ty
        });
    ty.is_reference() && expr.ir.return_type.is_reference() && actual == ty
}

fn dispatch_edge(
    target: BlockId,
    region: usize,
    state: &str,
    blocks: &HashSet<BlockId>,
    exits: &[(BlockId, RegionId)],
) -> Result<Body> {
    if blocks.contains(&target) {
        Ok(vec![
            Source::Simple(Simple::Dispatch {
                state: state.into(),
                target,
                declare: false,
            }),
            Source::Continue(Some(format!("$dispatch{region}"))),
        ])
    } else if let Some((_, label)) = exits.iter().find(|(block, _)| *block == target) {
        Ok(vec![Source::Break(Some(format!("label{label}")))])
    } else {
        bail!("dispatch edge to unrepresented block {target}")
    }
}

fn collect_labels(body: &[Stmt]) -> HashSet<RegionId> {
    use crate::structure::tree;
    let mut labels = HashSet::new();
    tree::walk_scoped(body, &mut |statement, active| match statement {
        Stmt::Label { region, .. } => {
            labels.insert(*region);
        }
        Stmt::Break { target } => {
            labels.extend(break_label(active, *target));
        }
        Stmt::Continue { header } => {
            labels.extend(continue_label(active, *header).ok().flatten());
        }
        Stmt::Dispatch { exits, .. } => labels.extend(exits.iter().map(|(_, region)| *region)),
        _ => (),
    });
    labels
}

fn break_label(active: &[Binder], target: RegionId) -> Option<RegionId> {
    // A dispatch introduces its own Java loop and switch; a plain label does
    // not intercept a bare break. Both label discovery and lowering use this.
    (!active
        .iter()
        .rev()
        .find(|binder| !matches!(binder, Binder::Label(_)))
        .is_some_and(|binder| !matches!(binder, Binder::Dispatch(_)) && binder.region() == target))
    .then_some(target)
}

fn continue_label(active: &[Binder], header: BlockId) -> Result<Option<RegionId>> {
    let nearest = active
        .iter()
        .rev()
        .find(|binder| matches!(binder, Binder::Loop { .. } | Binder::Dispatch(_)));
    if matches!(nearest, Some(Binder::Loop { header: current, .. }) if *current == header) {
        return Ok(None);
    }
    // Resolve the active occurrence when a recovered body contains lexical
    // copies of one bytecode loop, including a copy nested inside itself.
    active
        .iter()
        .rev()
        .find_map(|binder| match binder {
            Binder::Loop {
                region,
                header: current,
            } if *current == header => Some(Some(*region)),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("continue targets a loop outside its lexical scope"))
}

#[cfg(test)]
#[path = "../../../tests/unit/source_labels.rs"]
mod label_tests;
