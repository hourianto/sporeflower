//! Bind each scheduled operation as lowering inserts it into the source tree.
//! Preparation closes local storage and discards transient identity evidence.
use super::{Context, bound};
use crate::java::{expr::ExprPlanner, syntax::Expr};
use crate::{
    Result,
    classfile::JvmType,
    ir::{LocalId, Statement as Scheduled, ValueKind},
};
use anyhow::ensure;
use std::collections::BTreeSet;

pub(super) struct Binder<'a, 'b, 'c> {
    pub expr: &'c ExprPlanner<'a, 'b>,
    external: BTreeSet<LocalId>,
    storage: BTreeSet<LocalId>,
}
impl<'a, 'b, 'c> Binder<'a, 'b, 'c> {
    pub fn new(expr: &'c ExprPlanner<'a, 'b>, selected: Option<&BTreeSet<LocalId>>) -> Self {
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
        Self {
            expr,
            external,
            storage,
        }
    }

    pub fn finish(
        &self,
        body: &mut bound::Body,
        caught: &std::collections::HashSet<LocalId>,
        initialize: bool,
        initializer: bool,
    ) -> crate::java::source_locals::Declarations {
        let expr = self.expr;
        if !self.external.is_empty() {
            self.external_assignments(body);
        }
        super::evaluation::visit_expressions(body, |expression| {
            if let Some(details) = &mut expression.facts.details {
                details.origin = None;
                if !initializer {
                    details.publication = None;
                }
                if details.throws.is_empty() && details.publication.is_none() {
                    expression.facts.details = None;
                }
            }
            if matches!(&expression.kind, crate::java::syntax::Expression::Local {id, ..} if self.external.contains(id))
            {
                let value = std::mem::replace(expression, Expr::boolean(false));
                *expression = value.field();
            }
        });
        let live = expr
            .ir
            .locals
            .iter()
            .any(|local| local.needs_default)
            .then(|| crate::java::local_flow::live_at_entry(expr.ir).ok())
            .flatten();
        expr.ir
            .locals
            .iter_enumerated()
            .map(|(local, binding)| {
                if binding.is_this
                    || binding.parameter.is_some()
                    || self.external.contains(&local)
                    || caught.contains(&local)
                {
                    None
                } else {
                    Some(crate::java::source_locals::Declaration {
                        binding: self.binding(local),
                        initialize: initialize || binding.needs_default,
                        entry: initialize
                            || binding.needs_default
                                && live.as_ref().is_none_or(|live| live.contains(&local)),
                    })
                }
            })
            .collect::<Vec<_>>()
            .into()
    }
    fn external_assignments(&self, body: &mut bound::Body) {
        let convert = |simple: &mut bound::Simple| {
            if let bound::Simple::Assign { binding, value, .. } = simple
                && self.external.contains(&binding.local)
            {
                let target = binding.expression();
                let value =
                    std::mem::replace(value, bound::AssignmentValue::Value(Expr::boolean(false)));
                *simple = bound::Simple::Expression(value.into_expression(target));
            }
        };
        for statement in body {
            match statement {
                bound::Statement::Simple(simple) => convert(simple),
                bound::Statement::Loop {
                    header:
                        bound::LoopHeader::For {
                            initializer,
                            update,
                            ..
                        },
                    ..
                } => {
                    if let Some(initializer) = initializer {
                        convert(initializer);
                    }
                    convert(update);
                }
                _ => (),
            }
            super::evaluation::children(statement, |body| self.external_assignments(body));
        }
    }

    pub fn binding(&self, local: LocalId) -> bound::Binding {
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
    pub fn assignment(&self, local: LocalId, value: Expr, declaration: bool) -> bound::Simple {
        let binding = self.binding(local);
        bound::Simple::Assign {
            binding,
            value: bound::AssignmentValue::Value(value),
            declaration,
        }
    }

    fn update(&self, local: LocalId, update: Expr) -> bound::Simple {
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
    pub fn selector(
        &self,
        selector: crate::ir::ValueId,
        cases: &[super::Case],
    ) -> Result<bound::Selector> {
        let (expression, actual) = self.expr.typed_value(selector, &JvmType::Int)?;
        let character = actual.exactly(&JvmType::Char);
        let widen = actual.types().any(|ty| {
            crate::ir::type_facts::IntegralRange::of_type(ty).is_some_and(|range| {
                cases
                    .iter()
                    .flat_map(|case| &case.keys)
                    .any(|key| *key < range.min || *key > range.max)
            })
        });
        let value = if widen {
            Expr::cast("int", expression.grouped())
        } else {
            expression
        };
        let keys = cases
            .iter()
            .flat_map(|case| &case.keys)
            .map(|key| {
                let expression = if let Some(expression) = self.expr.switch_key(selector, *key) {
                    expression
                } else if character && let Ok(key) = u16::try_from(*key) {
                    Expr::atom(crate::java::literals::char_literal(key))
                } else {
                    Expr::atom(key.to_string())
                };
                (*key, expression)
            })
            .collect();
        Ok(bound::Selector { value, keys })
    }

    pub fn assign(
        &self,
        local: LocalId,
        usage: crate::ir::ValueUse,
        declaration: bool,
    ) -> Result<bound::Simple> {
        if !declaration
            && let Some(update) = crate::java::source_updates::assignment(self.expr, local, usage)?
        {
            return Ok(self.update(local, update));
        }
        let mut value = super::value(
            self.expr,
            usage,
            Context::Assignment(&self.expr.ir.locals[local].ty),
            false,
        )?;
        // Direct local identity already belongs to the expression. Only a
        // converted local or an exact literal definition needs extra evidence
        // while default factoring owns this assignment decision.
        if !matches!(
            value.kind,
            crate::java::syntax::Expression::Local { .. }
                | crate::java::syntax::Expression::Boolean(_)
        ) {
            let origin = usage
                .local(self.expr.ir)
                .map(crate::java::syntax::SourceLeaf::Local)
                .or_else(|| {
                    matches!(
                        self.expr.ir.values[usage.value()].kind,
                        ValueKind::Literal(_)
                    )
                    .then_some(crate::java::syntax::SourceLeaf::Literal(usage.value()))
                });
            if let Some(origin) = origin {
                value
                    .facts
                    .details
                    .get_or_insert_with(Default::default)
                    .origin = Some(origin);
            }
        }
        Ok(self.assignment(local, value, declaration))
    }

    pub fn catch_bind(&self, local: LocalId, name: &str, catch_type: &str) -> bound::Simple {
        self.assignment(
            local,
            Expr::atom(name).with_type(Some(&JvmType::Object(catch_type.into()))),
            false,
        )
    }

    pub fn scheduled(&self, statement: &Scheduled) -> Result<bound::Simple> {
        use crate::java::source_updates;
        let expression = match statement {
            Scheduled::Assign { local, value } => {
                return self.assign(*local, *value, false);
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
                        self.expr.array_element(*value, &ty)?,
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
        let mut expression = expression;
        if let Scheduled::PutField {
            field, receiver, ..
        } = statement
            && let Some(member) =
                self.expr
                    .cx
                    .program
                    .resolve_field(&field.owner, &field.name, &field.descriptor)
            && member.class == self.expr.cx.types.owner
            && (self.expr.cx.retains_final(member)
                || receiver.is_none()
                    && self.expr.member.is_some_and(|current| {
                        self.expr.cx.program.method(current).name == "<clinit>"
                    }))
        {
            let own_receiver = receiver.is_none_or(|receiver| {
                crate::java::constructors::is_this(
                    self.expr.ir,
                    receiver,
                    &mut std::collections::HashSet::new(),
                )
            });
            expression
                .facts
                .details
                .get_or_insert_with(Default::default)
                .publication = Some((member, own_receiver));
        }
        Ok(bound::Simple::Expression(expression))
    }
    pub fn throw(
        &self,
        value: crate::ir::ValueId,
        pc: u32,
        handled: &[String],
    ) -> Result<bound::Throw> {
        let fact = self.expr.member.and_then(|member| {
            self.expr
                .cx
                .program
                .throws_plan()
                .throw_at_instruction(member, pc)
        });
        let direct = fact.is_some_and(|fact| {
            handled
                .iter()
                .any(|ty| self.expr.cx.program.is_subtype(&fact.source_type, ty))
        });
        let mut effects = crate::java::exceptions::Thrown::default();
        crate::java::exceptions::throw_effects(
            self.expr.cx.program,
            self.expr.member,
            self.expr.ir,
            value,
            pc,
            &mut effects,
        );
        if direct && let Some(fact) = fact {
            // Runtime effects stay precise; the checker sees the static type
            // of this direct throw instead of a narrowing helper's signature.
            effects.source_types.insert(fact.source_type.clone());
        }
        Ok(bound::Throw {
            value: self.throw_value(value, fact, direct)?,
            effects,
        })
    }

    fn throw_value(
        &self,
        value: crate::ir::ValueId,
        fact: Option<&crate::throws::ThrowFact>,
        direct: bool,
    ) -> Result<Expr> {
        let mut helper = None;
        let expected = if let Some(fact) = fact {
            ensure!(
                direct
                    || !fact.unknown
                    || fact.bridge
                    || fact
                        .checked
                        .iter()
                        .any(|ty| { self.expr.cx.program.is_subtype(&fact.source_type, ty) }),
                "unbounded checked exception reaches an explicit cleanup rethrow; its finally or synchronized region must own this cleanup"
            );
            if fact.requires_narrowing && !direct {
                helper = Some(self.expr.cx.rethrow(&fact.checked, fact.bridge));
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

pub(super) fn invalid_dispatch() -> bound::Throw {
    bound::Throw {
        value: Expr::new("java.lang.Error", Vec::new()),
        effects: Default::default(),
    }
}

pub(super) fn dispatch_selector(state: &str, cases: &[super::Case]) -> bound::Selector {
    bound::Selector {
        value: Expr::atom(state),
        keys: cases
            .iter()
            .flat_map(|case| &case.keys)
            .map(|&key| (key, Expr::atom(key.to_string())))
            .collect(),
    }
}

pub(super) fn dispatch(state: &str, target: crate::ir::BlockId, declare: bool) -> bound::Simple {
    let value = Expr::assignment(Expr::atom(state), Expr::atom(target.to_string()), None);
    if declare {
        bound::Simple::Generated {
            type_name: "int".into(),
            value,
        }
    } else {
        bound::Simple::Expression(value)
    }
}
