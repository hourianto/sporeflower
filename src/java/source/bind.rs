//! Freeze semantic decisions into complete bound expressions, preserving the
//! structured statement tree for source rewrites before layout.
use super::{Body, Expression, LoopHeader, Selector, Simple, Statement, Throw, bound};
use crate::java::{expr::ExprPlanner, syntax::Expr};
use crate::{
    Result,
    classfile::JvmType,
    ir::{LocalId, Statement as Scheduled, ValueKind},
};
use anyhow::ensure;
use std::collections::BTreeSet;

pub(super) fn bind(
    expr: &ExprPlanner<'_, '_>,
    body: &Body,
    selected: Option<&BTreeSet<LocalId>>,
    caught: &std::collections::HashSet<LocalId>,
    initialize: bool,
) -> Result<(bound::Body, crate::java::source_locals::Declarations)> {
    // Excluded interface-helper bindings live in shared static fields. They
    // retain their source identity, but their accesses must have heap effects.
    let external = expr
        .ir
        .locals
        .iter_enumerated()
        .filter_map(|(local, binding)| {
            (selected.is_some_and(|selected| !selected.contains(&local))
                && binding.parameter.is_none()
                && !binding.is_this)
                .then_some(local)
        })
        .collect();
    let storage = expr
        .ir
        .values
        .iter()
        .filter_map(|value| {
            matches!(value.kind, ValueKind::Phi { slot: Some(_), .. })
                .then_some(value.local)
                .flatten()
        })
        .collect();
    let binder = Binder {
        expr,
        external,
        storage,
    };
    let mut body = binder.body(body)?;
    if !binder.external.is_empty() {
        super::evaluation::visit_expressions(&mut body, |expression| {
            if matches!(&expression.kind, crate::java::syntax::Expression::Local {id, ..} if binder.external.contains(id))
            {
                let value = std::mem::replace(expression, Expr::boolean(false));
                *expression = value.field();
            }
        });
    }
    let live = expr
        .ir
        .locals
        .iter()
        .any(|local| local.needs_default)
        .then(|| crate::java::local_flow::live_at_entry(expr.ir).ok())
        .flatten();
    let declarations = expr
        .ir
        .locals
        .iter_enumerated()
        .map(|(local, binding)| {
            if binding.is_this
                || binding.parameter.is_some()
                || binder.external.contains(&local)
                || caught.contains(&local)
            {
                None
            } else {
                Some(crate::java::source_locals::Declaration {
                    binding: binder.binding(local),
                    initialize: initialize || binding.needs_default,
                    entry: initialize
                        || binding.needs_default
                            && live.as_ref().is_none_or(|live| live.contains(&local)),
                })
            }
        })
        .collect::<Vec<_>>()
        .into();
    Ok((body, declarations))
}
struct Binder<'a, 'b, 'c> {
    expr: &'c ExprPlanner<'a, 'b>,
    external: BTreeSet<LocalId>,
    storage: BTreeSet<LocalId>,
}
impl Binder<'_, '_, '_> {
    fn binding(&self, local: LocalId) -> bound::Binding {
        let binding = &self.expr.ir.locals[local];
        bound::Binding {
            local,
            name: self.expr.local_name(local).to_owned(),
            ty: binding.ty.clone(),
            type_name: self.expr.cx.types.ty(&binding.ty),
            storage: self.storage.contains(&local),
            canonical_boolean: self.expr.canonical_local(local),
            synthetic: !self.external.contains(&local)
                && binding.name_hint.is_none()
                && binding.parameter.is_none()
                && !binding.is_this,
        }
    }
    fn assignment(&self, local: LocalId, value: Expr, declaration: bool) -> bound::Simple {
        let binding = self.binding(local);
        if self.external.contains(&local) {
            bound::Simple::Expression(Expr::assignment(binding.expression(), value, None))
        } else {
            bound::Simple::Assign {
                binding,
                value: bound::AssignmentValue::Value(value),
                declaration,
            }
        }
    }
    fn update(&self, local: LocalId, update: Expr) -> bound::Simple {
        if self.external.contains(&local) {
            return bound::Simple::Expression(update);
        }
        let value = match update.kind {
            crate::java::syntax::Expression::Update { increment, .. } => {
                bound::AssignmentValue::Increment { increment }
            }
            crate::java::syntax::Expression::Assignment {
                operands,
                compound: Some(operator),
            } => {
                let [_, value] = *operands;
                bound::AssignmentValue::Compound { operator, value }
            }
            _ => unreachable!("bound local update has an explicit write operation"),
        };
        bound::Simple::Assign {
            binding: self.binding(local),
            value,
            declaration: false,
        }
    }
    fn body(&self, body: &Body) -> Result<bound::Body> {
        body.iter()
            .map(|statement| self.statement(statement))
            .collect()
    }
    fn statement(&self, statement: &Statement) -> Result<bound::Statement> {
        Ok(match statement {
            Statement::Simple(value) => bound::Statement::Simple(self.simple(value)?),
            Statement::Return(value) => bound::Statement::Return(
                value
                    .as_ref()
                    .map(|value| value.plan(self.expr, &self.expr.ir.return_type))
                    .transpose()?,
            ),
            Statement::Throw(value) => bound::Statement::Throw(self.throw(value)?),
            Statement::Break(label) => bound::Statement::Break(label.clone()),
            Statement::Continue(label) => bound::Statement::Continue(label.clone()),
            Statement::Block { label, body } => bound::Statement::Block {
                label: label.clone(),
                body: self.body(body)?,
            },
            Statement::If { condition, yes, no } => bound::Statement::If {
                condition: condition.condition(self.expr)?,
                yes: self.body(yes)?,
                no: self.body(no)?,
            },
            Statement::Loop {
                label,
                header,
                body,
            } => bound::Statement::Loop {
                label: label.clone(),
                body: self.body(body)?,
                header: match header {
                    LoopHeader::While(value) => {
                        bound::LoopHeader::While(value.condition(self.expr)?)
                    }
                    LoopHeader::DoWhile(value) => {
                        bound::LoopHeader::DoWhile(value.condition(self.expr)?)
                    }
                    LoopHeader::For {
                        initializer,
                        condition,
                        update,
                    } => bound::LoopHeader::For {
                        initializer: initializer
                            .as_ref()
                            .map(|value| self.simple(value))
                            .transpose()?,
                        condition: condition.condition(self.expr)?,
                        update: Box::new(self.simple(update)?),
                    },
                },
            },
            Statement::Switch {
                label,
                value,
                cases,
            } => {
                let selector = match value {
                    Selector::Value(value) => Some(*value),
                    Selector::Dispatch(_) => None,
                };
                let (value, character) = match value {
                    Selector::Value(value) => {
                        let (expression, actual) = self.expr.typed_value(*value, &JvmType::Int)?;
                        let character = actual.exactly(&JvmType::Char);
                        let widen = actual.types().any(|ty| {
                            crate::ir::type_facts::IntegralRange::of_type(ty).is_some_and(|range| {
                                cases
                                    .iter()
                                    .flat_map(|case| &case.keys)
                                    .any(|key| *key < range.min || *key > range.max)
                            })
                        });
                        (
                            if widen {
                                Expr::cast("int", expression.grouped())
                            } else {
                                expression
                            },
                            character,
                        )
                    }
                    Selector::Dispatch(value) => (Expr::atom(value), false),
                };
                let keys = cases
                    .iter()
                    .flat_map(|case| &case.keys)
                    .map(|key| {
                        let expression = if let Some(expression) =
                            selector.and_then(|selector| self.expr.switch_key(selector, *key))
                        {
                            expression
                        } else if character && let Ok(key) = u16::try_from(*key) {
                            Expr::atom(crate::java::literals::char_literal(key))
                        } else {
                            Expr::atom(key.to_string())
                        };
                        (*key, expression)
                    })
                    .collect();
                bound::Statement::Switch {
                    label: label.clone(),
                    value: bound::Selector { value, keys },
                    cases: cases
                        .iter()
                        .map(|case| {
                            Ok(super::Case {
                                keys: case.keys.clone(),
                                default: case.default,
                                scoped: case.scoped,
                                body: self.body(&case.body)?,
                            })
                        })
                        .collect::<Result<_>>()?,
                }
            }
            Statement::Try {
                body,
                catches,
                cleanup,
            } => bound::Statement::Try {
                body: self.body(body)?,
                catches: catches
                    .iter()
                    .map(|catch| {
                        Ok(super::Catch {
                            local: catch.local,
                            catch_type: catch.catch_type.clone(),
                            ty: catch.ty.clone(),
                            name: catch.name.clone(),
                            body: self.body(&catch.body)?,
                        })
                    })
                    .collect::<Result<_>>()?,
                cleanup: cleanup.as_ref().map(|body| self.body(body)).transpose()?,
            },
            Statement::Synchronized { monitor, body } => bound::Statement::Synchronized {
                monitor: monitor.plan(self.expr, &JvmType::Object("java/lang/Object".into()))?,
                body: self.body(body)?,
            },
        })
    }
    fn simple(&self, statement: &Simple) -> Result<bound::Simple> {
        Ok(match statement {
            Simple::Scheduled(statement) => return self.scheduled(statement),
            Simple::Assign {
                local,
                value,
                declaration,
            } => {
                if !declaration
                    && let Expression::Value(value) = value
                    && let Some(update) =
                        crate::java::source_updates::assignment(self.expr, *local, *value)?
                {
                    return Ok(self.update(*local, update));
                }
                self.assignment(
                    *local,
                    value.plan(self.expr, &self.expr.ir.locals[*local].ty)?,
                    *declaration,
                )
            }
            Simple::CatchBind { local, name, ty } => self.assignment(
                *local,
                match ty {
                    Some(ty) => Expr::cast(ty, Expr::atom(name)),
                    None => Expr::atom(name),
                },
                false,
            ),
            Simple::Dispatch {
                state,
                target,
                declare,
            } => {
                let value =
                    Expr::assignment(Expr::atom(state), Expr::atom(target.to_string()), None);
                if *declare {
                    bound::Simple::Generated {
                        type_name: "int".into(),
                        value,
                    }
                } else {
                    bound::Simple::Expression(value)
                }
            }
        })
    }
    fn scheduled(&self, statement: &Scheduled) -> Result<bound::Simple> {
        use crate::java::source_updates;
        let expression = match statement {
            Scheduled::Assign { local, value } => {
                return self.simple(&Simple::Assign {
                    local: *local,
                    value: Expression::value(self.expr.ir, *value, &self.expr.ir.locals[*local].ty),
                    declaration: false,
                });
            }
            Scheduled::Eval(value) => {
                let expression = self.expr.statement_root(*value)?;
                if let ValueKind::LocalUpdate { local, .. } = self.expr.ir.values[*value].kind {
                    return Ok(self.update(local, expression));
                }
                if !matches!(
                    self.expr.ir.values[*value].kind,
                    ValueKind::Call { .. }
                        | ValueKind::New { .. }
                        | ValueKind::InitializeClass { .. }
                        | ValueKind::Update { .. }
                        | ValueKind::CompoundUpdate { .. }
                        | ValueKind::LocalUpdate { .. }
                ) {
                    return Ok(bound::Simple::Generated {
                        type_name: self.expr.cx.types.ty(&self.expr.ir.values[*value].ty),
                        value: Expr::assignment(
                            Expr::atom(format!("$discard{value}")),
                            expression,
                            None,
                        ),
                    });
                }
                expression
            }
            Scheduled::PutField {
                field,
                receiver,
                value,
                pc,
            } => {
                if let Some(update) = source_updates::field(self.expr, field, *receiver, *value)? {
                    update
                } else {
                    let mut value = self
                        .expr
                        .assigned(*value, &JvmType::parse(&field.descriptor)?)?;
                    if let Some(pc) = pc {
                        value = self.expr.class_name_field(*pc, value);
                    }
                    Expr::assignment(self.expr.field_assignment(field, *receiver)?, value, None)
                }
            }
            Scheduled::ArrayStore {
                array,
                index,
                value,
                element_type,
            } => {
                if let Some(update) =
                    source_updates::array(self.expr, *array, *index, *value, element_type)?
                {
                    update
                } else {
                    let (array, ty) =
                        self.expr
                            .array_store_receiver(*array, *value, element_type)?;
                    Expr::assignment(
                        array.index(self.expr.expected(*index, &JvmType::Int)?),
                        self.expr.assigned(*value, &ty)?,
                        None,
                    )
                }
            }
            Scheduled::ConstructorCall {
                method,
                receiver,
                args,
                pc,
            } => self.expr.constructor_call(method, *receiver, args, *pc)?,
            Scheduled::Monitor { .. } => {
                unreachable!("source lowering rejects unstructured monitors")
            }
        };
        Ok(bound::Simple::Expression(expression))
    }
    fn throw(&self, throwing: &Throw) -> Result<bound::Throw> {
        let mut effects = crate::java::exceptions::Thrown::default();
        if let Throw::Value { value, pc } = throwing {
            crate::java::exceptions::throw_effects(
                self.expr.cx.program,
                self.expr.member,
                self.expr.ir,
                *value,
                *pc,
                &mut effects,
            );
        }
        Ok(bound::Throw {
            value: self.throw_value(throwing)?,
            effects,
        })
    }

    fn throw_value(&self, throwing: &Throw) -> Result<Expr> {
        let (value, pc) = match throwing {
            Throw::InvalidDispatch => return Ok(Expr::new("java.lang.Error", Vec::new())),
            Throw::Value { value, pc } => (*value, *pc),
        };
        let fact = self.expr.member.and_then(|member| {
            self.expr
                .cx
                .program
                .throws_plan()
                .throw_at_instruction(member, pc)
        });
        let mut helper = None;
        let expected = if let Some(fact) = fact {
            ensure!(
                !fact.unknown,
                "unbounded checked exception reaches an explicit cleanup rethrow; its finally or synchronized region must own this cleanup"
            );
            if fact.requires_narrowing {
                helper = Some(self.expr.cx.rethrow(&fact.checked));
                JvmType::Object("java/lang/Throwable".into())
            } else {
                JvmType::Object(fact.source_type.clone())
            }
        } else {
            let ty = crate::java::conversions::source_type(self.expr.ir, value);
            if matches!(ty, JvmType::Object(name) if name != "java/lang/Object") {
                ty.clone()
            } else {
                JvmType::Object("java/lang/Throwable".into())
            }
        };
        let (value, actual) = self.expr.typed_value(value, &expected)?;
        Ok(if actual.null {
            Expr::cast("java.lang.RuntimeException", value)
        } else if let Some(helper) = helper {
            Expr::atom(helper).invoke(vec![value])
        } else {
            value
        })
    }
}
