use super::{
    conversions::SourceType,
    literals,
    syntax::{Expr, Operator, Prefix},
    types::TypeNames,
};
use crate::Result;
use crate::classfile::{JvmType, MethodDescriptor};
use crate::ir::UpdateTarget;
use crate::ir::{
    BinaryOp, CompareKind, CompareOp, Condition, InvokeKind, Literal, Member, MethodIr, UnaryOp,
    ValueId, ValueKind, ValueUse,
};
use crate::{
    naming::NamePlan,
    program::{MemberId, Program},
};
use anyhow::{bail, ensure};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Helper {
    CompareLong,
    CompareFloatLess,
    CompareFloatGreater,
    CompareDoubleLess,
    CompareDoubleGreater,
    ResourceLookup,
}

impl Helper {
    pub fn name(self) -> &'static str {
        match self {
            Self::CompareLong => "$sf$cmpLong",
            Self::CompareFloatLess => "$sf$cmpFloatL",
            Self::CompareFloatGreater => "$sf$cmpFloatG",
            Self::CompareDoubleLess => "$sf$cmpDoubleL",
            Self::CompareDoubleGreater => "$sf$cmpDoubleG",
            Self::ResourceLookup => "$sf$resource",
        }
    }
    pub fn source(self, name: &str, names: &NamePlan) -> String {
        let (ty, nan) = match self {
            Self::CompareLong => ("long", ""),
            Self::CompareFloatLess => ("float", "if (a != a || b != b) return -1; "),
            Self::CompareFloatGreater => ("float", "if (a != a || b != b) return 1; "),
            Self::CompareDoubleLess => ("double", "if (a != a || b != b) return -1; "),
            Self::CompareDoubleGreater => ("double", "if (a != a || b != b) return 1; "),
            Self::ResourceLookup => return super::resources::helper_source(name, names),
        };
        format!(
            "private static int {name}({ty} a, {ty} b) {{ {nan}return a < b ? -1 : a == b ? 0 : 1; }}"
        )
    }
}

pub struct ExprPlanner<'a, 'b> {
    pub cx: &'a super::context::EmissionContext<'a, 'b>,
    pub ir: &'a MethodIr,
    pub member: Option<MemberId>,
    /// The descriptor whose parameter indexes this body still uses. Extracted
    /// bodies retain `member` for original call resolution but clear this origin.
    pub(super) parameter_member: Option<MemberId>,
    pub(super) booleans: std::cell::OnceCell<super::boolean_values::CanonicalBooleans>,
    pub(super) semantics: std::cell::OnceCell<Option<crate::semantics::Analysis>>,
    pub(super) shared_semantics:
        Option<&'a std::cell::OnceCell<Option<crate::semantics::Analysis>>>,
    pub(super) semantic_diagnostics: std::cell::OnceCell<()>,
    pub(super) semantic_names:
        std::cell::OnceCell<Option<crate::index::IdVec<crate::ir::LocalId, String>>>,
}

impl<'a, 'b> ExprPlanner<'a, 'b> {
    /// Plan another expression from this method with independent support members.
    /// Interface fields share the frozen initializer's facts, but each direct
    /// initializer must discover its own helper requirements and diagnostics.
    pub(super) fn with_context<'s>(
        &'s self,
        cx: &'s super::context::EmissionContext<'s, 'b>,
    ) -> ExprPlanner<'s, 'b> {
        debug_assert!(std::ptr::eq(self.cx.program, cx.program));
        debug_assert!(
            self.cx.mappings.map(std::ptr::from_ref) == cx.mappings.map(std::ptr::from_ref)
        );
        let mut planner = cx.expressions(self.ir, self.member);
        planner.parameter_member = self.parameter_member;
        planner.shared_semantics = Some(self.shared_semantics.unwrap_or(&self.semantics));
        planner
    }
}

impl ExprPlanner<'_, '_> {
    pub(super) fn local_name(&self, local: crate::ir::LocalId) -> &str {
        if self.cx.semantic_local_names
            && let Some(analysis) = self.semantic_analysis()
            && let Some(names) = self.semantic_names.get_or_init(|| {
                super::local_names::semantic(self.ir, self.cx.program, self.cx.types, analysis)
            })
        {
            return &names[local];
        }
        &self.ir.locals[local].name
    }

    fn local_shadows(&self, name: &str) -> bool {
        self.ir
            .locals
            .ids()
            .any(|local| self.local_name(local) == name)
    }

    pub(super) fn semantic_analysis(&self) -> Option<&crate::semantics::Analysis> {
        let analysis = self
            .shared_semantics
            .unwrap_or(&self.semantics)
            .get_or_init(|| {
                let member = self.member?;
                Some(crate::semantics::Analysis::analyze(
                    self.ir,
                    self.cx.program,
                    self.cx.mappings?,
                    member,
                ))
            })
            .as_ref()?;
        self.semantic_diagnostics.get_or_init(|| {
            let member = self.member.expect("analyzed source member");
            let method = self.cx.program.method(member);
            self.cx
                .record_diagnostics(analysis.diagnostics.iter().map(|diagnostic| {
                    format!(
                        "{}.{}{}: {diagnostic}",
                        self.cx.program.class(member.class).name,
                        method.name,
                        method.descriptor
                    )
                }));
        });
        Some(analysis)
    }

    fn semantic_literal(&self, id: ValueId) -> Option<super::semantic::ConstantExpression> {
        let ValueKind::Literal(literal) = &self.ir.values[id].kind else {
            return None;
        };
        if self.ir.values[id].ty == JvmType::Boolean {
            return None;
        }
        let analysis = self.semantic_analysis()?;
        if analysis.class_name_literal(id)
            && let Literal::String(units) = literal
        {
            let original = String::from_utf16(units).ok()?;
            let mapped = relocated_class_name(&original, self.cx.types.names)?;
            return Some(super::semantic::ConstantExpression {
                expression: Expr::atom(literals::string_literal(
                    &mapped.encode_utf16().collect::<Vec<_>>(),
                )),
                numeric_type: None,
            });
        }
        let shadowed = |name: &str| self.local_shadows(name);
        let scope = super::semantic::Scope::Body(&shadowed);
        if let Some(constant) = analysis.packed_constant(id)
            && let Some(expression) = super::semantic::packed(self.cx, &constant, literal)
        {
            return Some(expression);
        }
        if let Some(constant) = analysis.offset_constant(id)
            && let Some(expression) = super::semantic::offset(self.cx, &constant, literal, scope)
        {
            return Some(expression);
        }
        super::semantic::literal(self.cx, &analysis.literal_domains(id), literal, scope)
    }

    pub(super) fn switch_key(&self, selector: ValueId, key: i32) -> Option<Expr> {
        let literal = Literal::Int(key);
        let shadowed = |name: &str| self.local_shadows(name);
        self.semantic_analysis()
            .and_then(|analysis| analysis.facts(selector, false).domain())
            .and_then(|domain| {
                super::semantic::literal(
                    self.cx,
                    &[domain],
                    &literal,
                    super::semantic::Scope::Body(&shadowed),
                )
            })
            .map(|constant| constant.expression)
            .or_else(|| {
                literals::standard_constant(self.cx.types, &literal, self.value_type(selector))
            })
    }

    pub(super) fn canonical_boolean(&self, usage: ValueUse) -> bool {
        self.boolean_facts().value(self.ir, usage)
    }

    pub(super) fn canonical_local(&self, local: crate::ir::LocalId) -> bool {
        self.ir.locals[local].ty == JvmType::Boolean && self.boolean_facts().local(self.ir, local)
    }

    fn boolean_facts(&self) -> &super::boolean_values::CanonicalBooleans {
        self.booleans.get_or_init(|| {
            super::boolean_values::CanonicalBooleans::build(
                self.ir,
                self.cx.program,
                self.member.map(|member| member.class),
                self.parameter_member,
            )
        })
    }

    /// A removable constant leaves the other eager operand evaluated once.
    /// The result says whether that operand must be inverted. AND/XOR with true
    /// additionally requires a 0/1 value to preserve its exact bits.
    pub(super) fn boolean_identity(
        &self,
        op: BinaryOp,
        operand: ValueId,
        constant: bool,
    ) -> Option<bool> {
        (matches!((op, constant), (BinaryOp::Or | BinaryOp::Xor, false))
            || matches!((op, constant), (BinaryOp::And | BinaryOp::Xor, true))
                && self.canonical_boolean(ValueUse::Reference(operand)))
        .then_some(op == BinaryOp::Xor && constant)
    }

    pub(super) fn boolean_binary(
        &self,
        op: BinaryOp,
        operands: [ValueId; 2],
        expressions: [Expr; 2],
    ) -> Expr {
        for index in 0..2 {
            let operand = operands[1 - index];
            if let Some(constant) = expressions[index].boolean_constant()
                && let Some(inverted) = self.boolean_identity(op, operand, constant)
            {
                let [left, right] = expressions;
                let operand = if index == 0 { right } else { left };
                return if inverted {
                    Expr::prefix(Prefix::Not, operand)
                } else {
                    operand
                };
            }
        }
        let [left, right] = expressions;
        Expr::binary(binary_operator(op), left, right).with_type(Some(&JvmType::Boolean))
    }

    pub fn root(&self, id: ValueId) -> Result<Expr> {
        self.expression(id, true, 0)
    }
    pub fn statement_root(&self, id: ValueId) -> Result<Expr> {
        // A concatenation is not a legal Java statement expression. Retain a
        // discarded invocation while still simplifying its receiver/arguments.
        if let ValueKind::Call {
            kind,
            method,
            receiver,
            args,
        } = &self.ir.values[id].kind
        {
            Ok(self
                .call(*kind, method, *receiver, args, self.ir.values[id].pc, 0)?
                .with_throws(super::exceptions::intrinsic_effects(
                    self.cx,
                    self.member,
                    self.ir,
                    id,
                )))
        } else {
            self.root(id)
        }
    }
    pub fn expected(&self, id: ValueId, expected: &JvmType) -> Result<Expr> {
        self.coerce(id, expected, false, 0)
    }
    pub(super) fn typed_value<'a>(
        &'a self,
        id: ValueId,
        expected: &'a JvmType,
    ) -> Result<(Expr, SourceType<'a>)> {
        self.coerced_with_type(id, expected, false, 0)
    }
    pub fn assigned(&self, id: ValueId, expected: &JvmType) -> Result<Expr> {
        self.assigned_at(id, expected, false, 0)
    }
    /// Source statement choices preserve the assignment conversion in each arm.
    /// Java 1.3 conditional expressions must not invent a numeric promotion or
    /// require a common reference type unavailable to that language version.
    pub(super) fn source_value(
        &self,
        usage: ValueUse,
        context: super::source::Context<'_>,
    ) -> Result<Expr> {
        let id = usage.value();
        let root = matches!(usage, ValueUse::Definition(_));
        let expected = context.ty();
        if matches!(context, super::source::Context::Assignment(_)) {
            return self.assigned_at(id, expected, root, 0);
        }
        let (expression, actual) = self.coerced_with_type(id, expected, root, 0)?;
        if !actual.exactly(expected)
            && !actual.null
            && ((actual.ty.is_reference() && expected.is_reference())
                || (super::conversions::numeric(actual.ty)
                    && super::conversions::numeric(expected)))
        {
            Ok(Expr::cast(self.cx.types.ty(expected), expression).with_type(Some(expected)))
        } else {
            Ok(expression)
        }
    }

    fn assigned_at(
        &self,
        id: ValueId,
        expected: &JvmType,
        root: bool,
        depth: usize,
    ) -> Result<Expr> {
        // Java assignment/return contexts permit representable int constants to
        // narrow implicitly. Invocation contexts still require the exact cast.
        if (root || self.ir.values[id].local.is_none())
            && let ValueKind::Literal(Literal::Int(value)) = self.ir.values[id].kind
            && match expected {
                JvmType::Byte => (-128..=127).contains(&value),
                JvmType::Short => (-32768..=32767).contains(&value),
                _ => false,
            }
        {
            return Ok(self.semantic_literal(id).map_or_else(
                || {
                    literals::standard_constant(self.cx.types, &Literal::Int(value), expected)
                        .unwrap_or_else(|| literal_expression(&Literal::Int(value)))
                },
                |constant| constant.expression,
            ));
        }
        if let Some(inner) = super::conversions::widened_operand(self.ir, id, expected, root) {
            return self.expression(inner, false, depth + 1);
        }
        self.coerce(id, expected, root, depth)
    }

    fn coerce(&self, id: ValueId, expected: &JvmType, root: bool, depth: usize) -> Result<Expr> {
        self.coerced(id, expected, root, depth)
    }

    fn coerced(&self, id: ValueId, expected: &JvmType, root: bool, depth: usize) -> Result<Expr> {
        Ok(self.coerced_with_type(id, expected, root, depth)?.0)
    }

    /// Preserve the source expression's type after conversion. Overload binding
    /// depends on this type, not the original JVM stack category.
    fn coerced_with_type<'a>(
        &'a self,
        id: ValueId,
        expected: &'a JvmType,
        root: bool,
        depth: usize,
    ) -> Result<(Expr, SourceType<'a>)> {
        let (expression, actual) = self.coerced_with_type_inner(id, expected, root, depth)?;
        Ok((
            expression.with_type(actual.exactly(actual.ty).then_some(actual.ty)),
            actual,
        ))
    }

    fn coerced_with_type_inner<'a>(
        &'a self,
        id: ValueId,
        expected: &'a JvmType,
        root: bool,
        depth: usize,
    ) -> Result<(Expr, SourceType<'a>)> {
        let value = &self.ir.values[id];
        let (expression, actual) = self.typed_expression(id, root, depth)?;
        if *expected == JvmType::Boolean {
            if *actual.ty == JvmType::Boolean {
                return Ok((expression, SourceType::value(expected)));
            }
            if (root || self.ir.values[id].local.is_none())
                && let ValueKind::Literal(Literal::Int(v)) = value.kind
            {
                return Ok((Expr::boolean(v != 0), SourceType::value(expected)));
            }
            return Ok((
                Expr::binary(Operator::NotEqual, expression, Expr::atom("0")),
                SourceType::value(expected),
            ));
        }
        if *actual.ty == JvmType::Boolean && expected.is_int_like() {
            let converted = Expr::conditional(expression, Expr::atom("1"), Expr::atom("0"));
            return Ok((
                if *expected == JvmType::Int {
                    converted
                } else {
                    Expr::cast(self.cx.types.ty(expected), converted)
                },
                SourceType::value(expected),
            ));
        }
        if let ValueKind::Literal(Literal::Int(v)) = value.kind
            && (self.ir.values[id].local.is_none() || root)
        {
            if *expected == JvmType::Char && (0..=65535).contains(&v) {
                let expression = if self.semantic_literal(id).is_some() {
                    if actual.exactly(expected) {
                        expression
                    } else {
                        Expr::cast("char", expression)
                    }
                } else {
                    Expr::atom(literals::char_literal(v as u16))
                };
                return Ok((expression, SourceType::value(expected)));
            }
            if matches!(expected, JvmType::Byte | JvmType::Short) {
                if self.semantic_literal(id).is_none()
                    && let Some(constant) =
                        literals::standard_constant(self.cx.types, &Literal::Int(v), expected)
                {
                    return Ok((constant, SourceType::value(expected)));
                }
                return Ok((
                    if actual.exactly(expected) {
                        expression
                    } else {
                        Expr::cast(self.cx.types.ty(expected), expression)
                    },
                    SourceType::value(expected),
                ));
            }
        }
        let cast = match (actual.ty, expected) {
            (
                JvmType::Int | JvmType::Byte | JvmType::Short | JvmType::Char,
                JvmType::Byte | JvmType::Short | JvmType::Char,
            ) => actual.ty != expected,
            (JvmType::Long, JvmType::Int | JvmType::Byte | JvmType::Short | JvmType::Char) => true,
            (
                JvmType::Float | JvmType::Double,
                JvmType::Int | JvmType::Byte | JvmType::Short | JvmType::Char | JvmType::Long,
            ) => true,
            (JvmType::Double, JvmType::Float) => true,
            (JvmType::Object(_) | JvmType::Array(_), JvmType::Object(_) | JvmType::Array(_)) => {
                !actual.null && !self.cx.program.assignable(actual.ty, expected)
            }
            _ => false,
        };
        if cast {
            let expression = if actual.ty.is_reference()
                && expected.is_reference()
                && !super::types::cast_compatible(
                    self.cx.program,
                    self.cx.types.names,
                    actual.ty,
                    expected,
                ) {
                Expr::cast("java.lang.Object", expression)
            } else {
                expression
            };
            Ok((
                Expr::cast(self.cx.types.ty(expected), expression),
                SourceType::value(expected),
            ))
        } else {
            Ok((expression, actual))
        }
    }

    fn expression(&self, id: ValueId, root: bool, depth: usize) -> Result<Expr> {
        Ok(self.typed_expression(id, root, depth)?.0)
    }

    fn conditional_constant(&self, value: ValueId) -> super::conversions::IntConstant {
        use super::conversions::IntConstant;
        let constant = super::conversions::int_constant(self.cx.program, self.ir, value);
        if !matches!(constant, IntConstant::Known(_)) {
            return constant;
        }
        // ConstantValue metadata does not make access through a typed-null
        // receiver a Java constant expression. Inspect the actual bound field
        // syntax, including an own field's possible unqualified spelling, so
        // conditional narrowing and overload planning follow rendered Java.
        let mut pending = vec![value];
        let mut seen = BTreeSet::new();
        while let Some(value) = pending.pop() {
            if !seen.insert(value) || self.ir.values[value].local.is_some() {
                continue;
            }
            let kind = &self.ir.values[value].kind;
            if let ValueKind::Field {
                field,
                receiver: None,
            } = kind
            {
                match self.field(field, None) {
                    Ok(reference) if !reference.is_named_reference() => {
                        return IntConstant::NotConstant;
                    }
                    Ok(_) => {}
                    Err(_) => return IntConstant::Unknown,
                }
            }
            pending.extend(kind.operands());
        }
        constant
    }

    fn typed_expression(
        &self,
        id: ValueId,
        root: bool,
        depth: usize,
    ) -> Result<(Expr, SourceType<'_>)> {
        ensure!(
            depth < 512,
            "expression nesting exceeds scheduled depth at value {id}"
        );
        if !root && let Some(local) = self.ir.values[id].local {
            return Ok((
                Expr::local(
                    local,
                    if self.ir.locals[local].is_this {
                        "this"
                    } else {
                        self.local_name(local)
                    },
                    &self.ir.locals[local].ty,
                )
                .with_canonical_boolean(
                    self.ir.locals[local].ty == JvmType::Boolean
                        && self.canonical_boolean(ValueUse::Reference(id)),
                ),
                SourceType::value(&self.ir.locals[local].ty),
            ));
        }
        let recurse = |value| self.expression(value, false, depth + 1);
        let value = &self.ir.values[id];
        let mut actual = SourceType::value(&value.ty);
        let expression = match &value.kind {
            ValueKind::Select {
                condition,
                then_value,
                else_value,
            } => {
                if value.ty == JvmType::Boolean {
                    return Ok((
                        super::source::value(
                            self,
                            ValueUse::Definition(id),
                            super::source::Context::Operand(&value.ty),
                            false,
                        )?
                        .with_canonical_boolean(self.canonical_boolean(ValueUse::Definition(id))),
                        actual,
                    ));
                }
                let (then_expression, then_type) =
                    self.coerced_with_type(*then_value, &value.ty, false, depth + 1)?;
                let (else_expression, else_type) =
                    self.coerced_with_type(*else_value, &value.ty, false, depth + 1)?;
                let mut then_text = then_expression;
                let mut else_text = else_expression;
                if let Some(ty) = super::conversions::conditional_source_type(
                    then_type,
                    self.conditional_constant(*then_value),
                    else_type,
                    self.conditional_constant(*else_value),
                ) {
                    actual = ty;
                }
                // Java 1.3 has no general reference LUB for a conditional. Give
                // one non-null arm the IR's proven common type when necessary.
                if value.ty.is_reference() {
                    actual.null = then_type.null && else_type.null;
                    if !then_type.null && then_type.ty != &value.ty {
                        then_text = Expr::cast(self.cx.types.ty(&value.ty), then_text.grouped());
                    } else if then_type.null && !else_type.null && else_type.ty != &value.ty {
                        else_text = Expr::cast(self.cx.types.ty(&value.ty), else_text.grouped());
                    }
                }
                Expr::conditional(
                    self.condition_expression(condition, false, false)?,
                    then_text,
                    else_text,
                )
            }
            ValueKind::InitializeClass { class } => {
                let owner = self.cx.program.id(class).ok_or_else(|| {
                    anyhow::anyhow!("initialization trigger has no own class {class}")
                })?;
                let helper = self
                    .cx
                    .types
                    .names
                    .initialization_helper(owner)
                    .ok_or_else(|| {
                        anyhow::anyhow!("class {class} has no planned initialization trigger")
                    })?;
                let target = self.cx.types.qualifier(class);
                if self.cx.program.class(owner).access_flags & 0x0200 != 0 {
                    target
                        .member(format!("{helper}$Trigger"))
                        .member("initialize")
                        .invoke(Vec::new())
                } else {
                    target.member(helper).invoke(Vec::new())
                }
            }
            ValueKind::ArrayInitializer {
                element_type,
                values,
                ..
            } => self.array_initializer(element_type, values, depth, true)?,
            ValueKind::InlineAssign { local, value } => {
                let target =
                    Expr::local(*local, self.local_name(*local), &self.ir.locals[*local].ty);
                let local = &self.ir.locals[*local];
                actual = SourceType::value(&local.ty);
                Expr::assignment(
                    target,
                    self.assigned_at(*value, &local.ty, true, depth + 1)?,
                    None,
                )
                .grouped()
            }
            ValueKind::LocalUpdate {
                local,
                increment,
                postfix,
                ..
            } => {
                let target =
                    Expr::local(*local, self.local_name(*local), &self.ir.locals[*local].ty);
                let local = &self.ir.locals[*local];
                actual = SourceType::value(&local.ty);
                Expr::update(target, *increment, *postfix)
            }
            ValueKind::Update {
                target,
                increment,
                postfix,
            } => {
                let target = match target {
                    UpdateTarget::Field { field, receiver } => self.field(field, *receiver)?,
                    UpdateTarget::Array {
                        array,
                        index,
                        element_type,
                    } => {
                        let receiver = self.receiver(
                            *array,
                            &JvmType::Array(Box::new(element_type.clone())),
                            depth + 1,
                        )?;
                        receiver.index(self.coerce(*index, &JvmType::Int, false, depth + 1)?)
                    }
                };
                Expr::update(target, *increment, *postfix)
            }
            ValueKind::CompoundUpdate {
                target,
                op,
                right,
                operation_type,
            } => {
                let place = match target {
                    UpdateTarget::Field { field, receiver } => {
                        self.field_assignment(field, *receiver)?
                    }
                    UpdateTarget::Array {
                        array,
                        index,
                        element_type,
                    } => self
                        .array_receiver(*array, element_type)?
                        .index(self.expected(*index, &JvmType::Int)?),
                };
                super::source_updates::plan_operation(
                    self,
                    place,
                    &value.ty,
                    *op,
                    *right,
                    operation_type,
                )?
            }
            ValueKind::Alias(value) => return self.typed_expression(*value, false, depth + 1),
            ValueKind::Parameter { slot, index } => Expr::atom(if index.is_none() {
                "this".to_owned()
            } else {
                format!("p{slot}")
            }),
            ValueKind::Literal(Literal::Class(ty)) => {
                Expr::atom(self.cx.types.ty(ty)).member("class")
            }
            ValueKind::Literal(literal) => {
                actual.null = matches!(literal, Literal::Null);
                if let Some(constant) = self.semantic_literal(id) {
                    if let Some(ty) = constant.numeric_type {
                        actual.ty = ty;
                    }
                    constant.expression
                } else if let Literal::Int(v) = literal {
                    if value.ty == JvmType::Boolean {
                        Expr::boolean(*v != 0)
                    } else {
                        actual.ty = &JvmType::Int;
                        literals::standard_constant(self.cx.types, literal, actual.ty)
                            .unwrap_or_else(|| literal_expression(literal))
                    }
                } else {
                    literals::standard_constant(self.cx.types, literal, actual.ty)
                        .unwrap_or_else(|| literal_expression(literal))
                }
            }
            ValueKind::Unary {
                op: UnaryOp::Neg,
                value,
            } => {
                let value = if self.ir.values[*value].ty == JvmType::Boolean {
                    self.coerced(*value, &JvmType::Int, false, depth + 1)?
                } else {
                    recurse(*value)?
                };
                Expr::prefix(Prefix::Negate, value)
            }
            ValueKind::Binary { op, left, right } => {
                let boolean = matches!(op, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor)
                    && value.ty == JvmType::Boolean;
                let operands = [*left, *right];
                let (left, right) =
                    if !boolean && !matches!(op, BinaryOp::Shl | BinaryOp::Shr | BinaryOp::Ushr) {
                        super::conversions::binary_operands(self.ir, *left, *right, &value.ty)
                    } else {
                        (*left, *right)
                    };
                let additive = if self.semantic_literal(right).is_some() {
                    None
                } else {
                    literals::additive_operand(self.ir, *op, right, &value.ty)
                };
                let operator = binary_operator(additive.as_ref().map_or(*op, |(op, _)| *op));
                let left = if boolean {
                    self.coerced(left, &JvmType::Boolean, false, depth + 1)?
                } else if self.value_type(left) == &JvmType::Boolean {
                    self.coerced(left, &JvmType::Int, false, depth + 1)?
                } else {
                    recurse(left)?
                };
                let right = if let Some((_, right)) = additive {
                    literal_expression(&right)
                } else if boolean {
                    self.coerced(right, &JvmType::Boolean, false, depth + 1)?
                } else if self.value_type(right) == &JvmType::Boolean {
                    self.coerced(right, &JvmType::Int, false, depth + 1)?
                } else {
                    recurse(right)?
                };
                if boolean {
                    self.boolean_binary(*op, operands, [left, right])
                } else {
                    Expr::binary(operator, left, right)
                }
            }
            ValueKind::Compare { kind, left, right } => {
                let helper = match kind {
                    CompareKind::Long => Helper::CompareLong,
                    CompareKind::FloatLess => Helper::CompareFloatLess,
                    CompareKind::FloatGreater => Helper::CompareFloatGreater,
                    CompareKind::DoubleLess => Helper::CompareDoubleLess,
                    CompareKind::DoubleGreater => Helper::CompareDoubleGreater,
                };
                let name = self.cx.helper(helper);
                let ty = compare_type(*kind);
                Expr::atom(name).invoke(vec![
                    self.assigned_at(*left, &ty, false, depth + 1)?,
                    self.assigned_at(*right, &ty, false, depth + 1)?,
                ])
            }
            ValueKind::Cast { value, ty } => {
                let value = if self.ir.values[*value].ty == JvmType::Boolean
                    && !ty.is_reference()
                    && *ty != JvmType::Boolean
                {
                    self.coerced(*value, &JvmType::Int, false, depth + 1)?
                } else if ty.is_reference()
                    && !super::types::cast_compatible(
                        self.cx.program,
                        self.cx.types.names,
                        self.value_type(*value),
                        ty,
                    )
                {
                    Expr::cast("java.lang.Object", recurse(*value)?)
                } else {
                    recurse(*value)?
                };
                Expr::cast(self.cx.types.ty(ty), value)
            }
            ValueKind::InstanceOf { value, ty } => {
                let value = if super::types::cast_compatible(
                    self.cx.program,
                    self.cx.types.names,
                    self.value_type(*value),
                    ty,
                ) {
                    recurse(*value)?
                } else {
                    Expr::cast("java.lang.Object", recurse(*value)?)
                };
                Expr::instance_of(value, self.cx.types.ty(ty))
            }
            ValueKind::Field { field, receiver } => self.field(field, *receiver)?,
            ValueKind::ArrayLoad {
                array,
                index,
                element_type,
            } => {
                actual.ty = element_type;
                let array_ty = JvmType::Array(Box::new(element_type.clone()));
                let (array_text, array_type) = self.typed_expression(*array, false, depth + 1)?;
                // A shared aaload can be projected into arms with narrower
                // arrays. Its Java type follows the emitted receiver, while a
                // materialized array still has its local's declaration type.
                if !array_type.null
                    && self.cx.program.assignable(array_type.ty, &array_ty)
                    && let JvmType::Array(component) = array_type.ty
                {
                    actual.ty = component;
                }
                let array_text = self.receiver_expression(array_text, array_type, &array_ty);
                array_text.index(self.coerce(*index, &JvmType::Int, false, depth + 1)?)
            }
            ValueKind::ArrayLength { array } => {
                let (expression, actual) = self.typed_expression(*array, false, depth + 1)?;
                let receiver = if actual.null {
                    Expr::cast("java.lang.Object[]", expression)
                } else {
                    expression
                };
                receiver.member("length").field()
            }
            ValueKind::Call {
                kind,
                method,
                receiver,
                args,
            } => {
                if let Some(concatenation) =
                    super::string_concat::recognize(self.cx.program, self.ir, id)
                {
                    self.concatenation(&concatenation, depth + 1)?
                } else {
                    self.call(
                        *kind,
                        method,
                        *receiver,
                        args,
                        self.ir.values[id].pc,
                        depth + 1,
                    )?
                    .with_throws(super::exceptions::intrinsic_effects(
                        self.cx,
                        self.member,
                        self.ir,
                        id,
                    ))
                }
            }
            ValueKind::New {
                class,
                descriptor,
                args,
                ..
            } => {
                let descriptor = MethodDescriptor::parse(descriptor)?;
                let mut args = self.arguments(
                    args,
                    &descriptor.parameters,
                    &super::overloads::Site {
                        owners: &[class.as_str()],
                        name: "<init>",
                        constructor: true,
                    },
                    depth + 1,
                )?;
                self.class_name_arguments(self.ir.values[id].pc, &mut args);
                Expr::new(self.cx.types.class(class), args).with_throws(
                    super::exceptions::intrinsic_effects(self.cx, self.member, self.ir, id),
                )
            }
            ValueKind::NewArray {
                element_type,
                dimensions,
            } => {
                let mut element = element_type;
                let mut trailing = 0;
                while let JvmType::Array(inner) = element {
                    trailing += 1;
                    element = inner;
                }
                let dimensions = dimensions
                    .iter()
                    .map(|dimension| self.coerce(*dimension, &JvmType::Int, false, depth + 1))
                    .collect::<Result<Vec<_>>>()?;
                Expr::new_array(self.cx.types.ty(element), dimensions, trailing)
            }
            ValueKind::CaughtException { handler } => {
                bail!("unscheduled caught exception at handler {handler}")
            }
            ValueKind::Allocation { .. } => {
                bail!("uninitialized allocation reached Java expression planning")
            }
            ValueKind::Phi { .. } => {
                bail!("unscheduled phi value {id} reached Java expression planning")
            }
        };
        let expression = expression.with_canonical_boolean(
            actual.ty == &JvmType::Boolean && self.canonical_boolean(ValueUse::Definition(id)),
        );
        let expression = match value.kind {
            ValueKind::Literal(Literal::Int(value)) if actual.ty != &JvmType::Boolean => {
                expression.with_integer_value(i64::from(value))
            }
            ValueKind::Literal(Literal::Long(value)) => expression.with_integer_value(value),
            _ => expression,
        };
        Ok((
            expression.with_type(actual.exactly(actual.ty).then_some(actual.ty)),
            actual,
        ))
    }

    fn concatenation(
        &self,
        concatenation: &super::string_concat::Concatenation,
        depth: usize,
    ) -> Result<Expr> {
        let mut parts = Vec::with_capacity(concatenation.parts.len() + 1);
        if concatenation.empty_prefix {
            parts.push(Expr::atom("\"\""));
        }
        for part in &concatenation.parts {
            let (mut expression, actual) =
                self.coerced_with_type(part.value, &part.ty, false, depth + 1)?;
            // An int append of a char prints its numeric code.
            // Floating-point append overloads likewise determine conversion and
            // formatting. Preserve Object overloads on arrays explicitly.
            let cast = match (&part.ty, actual.ty) {
                (JvmType::Int | JvmType::Long, _)
                    if actual.types().any(|ty| *ty == JvmType::Char) =>
                {
                    Some(&JvmType::Int)
                }
                (JvmType::Float | JvmType::Double, actual) if actual != &part.ty => Some(&part.ty),
                (JvmType::Object(name), JvmType::Array(_)) if name == "java/lang/Object" => {
                    Some(&part.ty)
                }
                _ => None,
            };
            if let Some(ty) = cast {
                expression = Expr::cast(self.cx.types.ty(ty), expression);
            }
            parts.push(expression);
        }
        // Keep the append sequence visible as a sequence of source operands.
        // Long chains are flat even when their bytecode receiver spine is deep.
        let expression = Expr::concatenation(parts);
        Ok(if concatenation.fresh_wrapper {
            Expr::new(self.cx.types.class("java/lang/String"), vec![expression])
        } else {
            expression
        })
    }

    fn array_initializer(
        &self,
        element_type: &JvmType,
        values: &[ValueId],
        depth: usize,
        explicit_type: bool,
    ) -> Result<Expr> {
        ensure!(
            depth < 512,
            "nested array initializer exceeds scheduled depth"
        );
        let elements = values
            .iter()
            .map(|&value| {
                if self.ir.values[value].local.is_none()
                    && &self.ir.values[value].ty == element_type
                    && let ValueKind::ArrayInitializer {
                        element_type,
                        values,
                        ..
                    } = &self.ir.values[value].kind
                {
                    self.array_initializer(element_type, values, depth + 1, false)
                } else {
                    self.assigned_at(value, element_type, false, depth + 1)
                }
            })
            .collect::<Result<Vec<_>>>()?;
        let head = if explicit_type {
            format!("new {}[] {{", self.cx.types.ty(element_type))
        } else {
            "{".into()
        };
        Ok(Expr::array_initializer(
            head,
            elements,
            matches!(element_type, JvmType::Array(_)),
        ))
    }

    fn arguments(
        &self,
        args: &[ValueId],
        parameters: &[JvmType],
        site: &super::overloads::Site<'_>,
        depth: usize,
    ) -> Result<Vec<Expr>> {
        self.arguments_with_first(args, parameters, site, depth, None)
    }

    fn arguments_with_first(
        &self,
        args: &[ValueId],
        parameters: &[JvmType],
        site: &super::overloads::Site<'_>,
        depth: usize,
        first: Option<&str>,
    ) -> Result<Vec<Expr>> {
        ensure!(args.len() == parameters.len(), "argument arity mismatch");
        let planned = args
            .iter()
            .zip(parameters)
            .enumerate()
            .map(|(index, (&id, ty))| {
                if index == 0
                    && let Some(first) = first
                    && self.semantic_literal(id).is_none()
                {
                    return Ok((Expr::atom(first), SourceType::value(&JvmType::Char)));
                }
                let id = super::conversions::widened_operand(self.ir, id, ty, false).unwrap_or(id);
                let (expression, actual) = self.coerced_with_type(id, ty, false, depth + 1)?;
                Ok((expression, actual))
            })
            .collect::<Result<Vec<_>>>()?;
        let arguments = planned
            .iter()
            .map(|(_, actual)| *actual)
            .collect::<Vec<_>>();
        let mut casts = super::overloads::casts(
            self.cx.program,
            self.cx.types.names,
            self.cx.types.owner,
            site,
            parameters,
            &arguments,
        );
        // The narrowly specified String character APIs have no char overload;
        // the literal's widening remains safe even without a loaded API catalog.
        if first.is_some() {
            casts[0] = false;
        }
        Ok(planned
            .into_iter()
            .zip(parameters)
            .zip(casts)
            .map(|(((expression, _), ty), cast)| {
                if cast {
                    Expr::cast(self.cx.types.ty(ty), expression)
                } else {
                    expression
                }
            })
            .collect::<Vec<_>>())
    }

    fn receiver(&self, id: ValueId, expected: &JvmType, depth: usize) -> Result<Expr> {
        let (expression, actual) = self.typed_expression(id, false, depth)?;
        Ok(self.receiver_expression(expression, actual, expected))
    }

    fn receiver_expression(
        &self,
        expression: Expr,
        actual: SourceType<'_>,
        expected: &JvmType,
    ) -> Expr {
        if actual.null {
            Expr::cast(self.cx.types.ty(expected), expression)
        } else if self.cx.program.assignable(actual.ty, expected) {
            expression
        } else {
            let expression = if !super::types::cast_compatible(
                self.cx.program,
                self.cx.types.names,
                actual.ty,
                expected,
            ) {
                Expr::cast("java.lang.Object", expression)
            } else {
                expression
            };
            Expr::cast(self.cx.types.ty(expected), expression)
        }
    }

    pub fn array_receiver(&self, value: ValueId, element_type: &JvmType) -> Result<Expr> {
        self.receiver(value, &JvmType::Array(Box::new(element_type.clone())), 0)
    }

    pub fn array_store_receiver(
        &self,
        array: ValueId,
        value: ValueId,
        element_type: &JvmType,
    ) -> Result<(Expr, JvmType)> {
        if !element_type.is_reference() {
            return Ok((
                self.array_receiver(array, element_type)?,
                element_type.clone(),
            ));
        }
        let object = JvmType::Object("java/lang/Object".into());
        let component = match self.value_type(array) {
            JvmType::Array(component) => component.as_ref(),
            _ => &object,
        };
        let null = super::conversions::null_literal(self.ir, value, false);
        let receiver = if null
            || self
                .cx
                .program
                .assignable(self.value_type(value), component)
        {
            self.array_receiver(array, &object)?
        } else {
            // Widen the array view instead of narrowing its operand. AASTORE
            // must perform null/bounds/component checks after RHS evaluation.
            Expr::cast("java.lang.Object[]", self.expression(array, false, 0)?)
        };
        Ok((receiver, object))
    }

    pub fn field(&self, field: &Member, receiver: Option<ValueId>) -> Result<Expr> {
        Ok(self.bound_field(field, receiver)?.field())
    }

    fn bound_field(&self, field: &Member, receiver: Option<ValueId>) -> Result<Expr> {
        let name = self.cx.types.names.resolved_field_name(
            self.cx.program,
            &field.owner,
            &field.name,
            &field.descriptor,
        );
        let declaration =
            self.cx
                .program
                .resolve_field(&field.owner, &field.name, &field.descriptor);
        // A field declared here wins inherited lookup, but a source local can
        // still shadow it. Keep the qualifier whenever either binding is unsure.
        if receiver.is_none()
            && declaration.is_some_and(|member| {
                let field = self.cx.program.field(member);
                // Qualified reads can observe a blank final's VM default in
                // its initializer. A simple name would require Java definite
                // assignment, even though the bytecode read is well-defined.
                let blank_final_read = field.access_flags & 0x0010 != 0
                    && field.constant_value.is_none()
                    && self
                        .member
                        .is_some_and(|current| self.cx.program.method(current).name == "<clinit>");
                member.class == self.cx.types.owner
                    && field.access_flags & 8 != 0
                    && !blank_final_read
            })
            && !self.local_shadows(name)
        {
            return Ok(Expr::atom(name));
        }
        let owner = declaration
            .filter(|member| {
                let declaring = self.cx.program.class(member.class);
                declaring.access_flags & 0x0001 != 0
                    || self
                        .cx
                        .types
                        .names
                        .class_name(member.class)
                        .rsplit_once('/')
                        .map(|(p, _)| p)
                        == self
                            .cx
                            .types
                            .names
                            .class_name(self.cx.types.owner)
                            .rsplit_once('/')
                            .map(|(p, _)| p)
            })
            .map(|member| self.cx.program.class(member.class).name.as_str())
            .unwrap_or(&field.owner);
        let qualifier = match receiver {
            Some(value) => {
                let actual = self.value_type(value);
                let shadowed = match (actual, declaration) {
                    (JvmType::Object(actual), Some(target)) => {
                        self.cx.program.id(actual).is_some_and(|actual| {
                            actual != target.class
                                && self.cx.program.ancestors(actual).contains(&target.class)
                                && !self.source_field_resolves(actual, name, target)
                        })
                    }
                    _ => false,
                };
                let super_resolves = declaration.is_some_and(|target| {
                    let mut current = self
                        .cx
                        .program
                        .class(self.cx.types.owner)
                        .super_name
                        .as_deref()
                        .and_then(|name| self.cx.program.id(name));
                    while let Some(class) = current {
                        if let Some((index, _)) =
                            self.cx.program.class(class).fields.iter().enumerate().find(
                                |(index, _)| {
                                    self.cx.types.names.field_name(crate::program::MemberId {
                                        class,
                                        index: *index,
                                    }) == name
                                },
                            )
                        {
                            return target == crate::program::MemberId { class, index };
                        }
                        current = self
                            .cx
                            .program
                            .class(class)
                            .super_name
                            .as_deref()
                            .and_then(|name| self.cx.program.id(name));
                    }
                    false
                });
                if let Some(target) = declaration.filter(|&target| {
                    self.cx.program.field(target).access_flags & 0x0002 != 0
                        && actual
                            != &JvmType::Object(self.cx.program.class(target.class).name.clone())
                }) {
                    // Private members are not inherited. Even inside their
                    // declaring class, Java needs that class's receiver view
                    // rather than a subtype view to select the same field.
                    Expr::cast(
                        self.cx
                            .types
                            .class(&self.cx.program.class(target.class).name),
                        self.expression(value, false, 0)?,
                    )
                } else if shadowed
                    && self.proven_this(value, &mut BTreeSet::new())
                    && super_resolves
                {
                    Expr::atom("super")
                } else if shadowed {
                    let owner = declaration
                        .filter(|&target| {
                            self.cx.program.id(&field.owner).is_some_and(|owner| {
                                self.source_field_resolves(owner, name, target)
                            })
                        })
                        .map_or(owner, |_| field.owner.as_str());
                    Expr::cast(
                        self.cx.types.class(owner),
                        self.expression(value, false, 0)?,
                    )
                } else {
                    self.receiver(value, &JvmType::Object(owner.to_owned()), 0)?
                }
            }
            None => self.cx.types.qualifier(owner),
        };
        Ok(qualifier.member(name))
    }

    /// A declaration hides fields above it, not below it. Looking through the
    /// resolved declaration can invent a cast which also makes a protected
    /// subclass receiver illegal in Java source.
    fn source_field_resolves(&self, owner: usize, name: &str, target: MemberId) -> bool {
        self.cx
            .types
            .names
            .source_field_resolves(self.cx.program, owner, name, target)
    }

    /// Blank finals have narrower Java assignment syntax than ordinary fields.
    /// A static blank final uses its simple name in its declaring initializer;
    /// an instance blank final must use the constructor's actual `this` receiver.
    pub fn field_assignment(&self, field: &Member, receiver: Option<ValueId>) -> Result<Expr> {
        if let Some(target) =
            self.cx
                .program
                .resolve_field(&field.owner, &field.name, &field.descriptor)
            && target.class == self.cx.types.owner
            && self.cx.program.field(target).access_flags & 0x0010 != 0
        {
            let name = self.cx.types.names.field_name(target);
            if receiver.is_none() {
                return Ok(Expr::atom(name).field());
            }
            if receiver.is_some_and(|value| self.proven_this(value, &mut BTreeSet::new())) {
                return Ok(Expr::atom("this").member(name).field());
            }
        }
        self.field(field, receiver)
    }

    fn call(
        &self,
        kind: InvokeKind,
        method: &Member,
        receiver: Option<ValueId>,
        args: &[ValueId],
        offset: u32,
        depth: usize,
    ) -> Result<Expr> {
        let generated = self
            .cx
            .generated
            .borrow()
            .get(&method.owner, &method.name, &method.descriptor)
            .cloned();
        let descriptor = generated.as_ref().map_or_else(
            || MethodDescriptor::parse(&method.descriptor),
            |contract| Ok(contract.signature.clone()),
        )?;
        let name = if generated.is_some() {
            method.name.as_str()
        } else {
            self.cx.types.names.resolved_method_name(
                self.cx.program,
                &method.owner,
                &method.name,
                &method.descriptor,
            )
        };
        let mut owners = vec![method.owner.as_str()];
        if let Some(receiver) = receiver
            && let JvmType::Object(owner) = self.value_type(receiver)
            && owner != &method.owner
        {
            owners.push(owner.as_str());
        }
        let character = (kind == InvokeKind::Virtual)
            .then(|| literals::character_argument(method, self.ir, args))
            .flatten();
        if let Some(contract) = generated {
            ensure!(
                args.len() == descriptor.parameters.len(),
                "generated argument arity mismatch"
            );
            let arguments = args
                .iter()
                .zip(&descriptor.parameters)
                .map(|(&value, ty)| self.coerce(value, ty, false, depth + 1))
                .collect::<Result<Vec<_>>>()?;
            let qualifier = if let Some(container) = contract.container {
                Expr::atom(container)
            } else if let Some(receiver) = receiver {
                self.receiver(receiver, &JvmType::Object(method.owner.clone()), depth + 1)?
            } else {
                self.cx.types.qualifier(&method.owner)
            };
            return Ok(qualifier.member(name).invoke(arguments));
        }
        let mut arguments = self.arguments_with_first(
            args,
            &descriptor.parameters,
            &super::overloads::Site {
                owners: &owners,
                name,
                constructor: false,
            },
            depth + 1,
            character.as_deref(),
        )?;
        self.class_name_arguments(offset, &mut arguments);

        if kind == InvokeKind::Static
            && method.owner == "java/lang/Class"
            && method.name == "forName"
            && method.descriptor == "(Ljava/lang/String;)Ljava/lang/Class;"
            && self.cx.mappings.is_none()
            && let [value] = args
            && let ValueKind::Literal(Literal::String(units)) = &self.ir.values[*value].kind
            && let Ok(original) = String::from_utf16(units)
        {
            let mapped = relocated_class_name(&original, self.cx.types.names);
            if let Some(mapped) = mapped.filter(|mapped| mapped != &original) {
                arguments = vec![Expr::atom(literals::string_literal(
                    &mapped.encode_utf16().collect::<Vec<_>>(),
                ))];
            }
        }
        if kind == InvokeKind::Static {
            return Ok(self
                .cx
                .types
                .qualifier(&method.owner)
                .member(name)
                .invoke(arguments));
        }
        let receiver =
            receiver.ok_or_else(|| anyhow::anyhow!("instance invocation without receiver"))?;
        if method.owner == "java/lang/Class"
            && method.name == "getResourceAsStream"
            && method.descriptor == "(Ljava/lang/String;)Ljava/io/InputStream;"
            && !self.cx.types.names.package_moves().is_empty()
            && args.first().is_none_or(|&path| {
                super::resources::needs_relocation(self.cx.program, self.ir, path)
            })
        {
            let name = self.cx.helper(Helper::ResourceLookup);
            let receiver = self.coerce(
                receiver,
                &JvmType::Object("java/lang/Class".into()),
                false,
                depth + 1,
            )?;
            arguments.insert(0, receiver);
            return Ok(Expr::atom(name).invoke(arguments));
        }
        if kind == InvokeKind::Special
            && let Some(helper) = self
                .cx
                .program
                .resolve_method(&method.owner, &method.name, &method.descriptor)
                .and_then(|target| self.cx.special.helper(target))
        {
            return Ok(self
                .receiver(receiver, &JvmType::Object(method.owner.clone()), depth + 1)?
                .member(helper)
                .invoke(arguments));
        }
        if kind == InvokeKind::Special && self.proven_this(receiver, &mut BTreeSet::new()) {
            let declaration =
                self.cx
                    .program
                    .resolve_method(&method.owner, &method.name, &method.descriptor);
            let current = self.cx.program.class(self.cx.types.owner);
            let superclass = declaration.map_or(method.owner != current.name, |target| {
                target.class != self.cx.types.owner
            });
            if superclass {
                return Ok(Expr::atom("super").member(name).invoke(arguments));
            }
        }
        let declaration =
            self.cx
                .program
                .resolve_method(&method.owner, &method.name, &method.descriptor);
        let source_target = match self.value_type(receiver) {
            JvmType::Object(owner) => {
                self.cx
                    .program
                    .resolve_method(owner, &method.name, &method.descriptor)
            }
            _ => None,
        };
        let private_owner =
            declaration.filter(|&target| self.cx.program.method(target).access_flags & 0x0002 != 0);
        let qualifier = if let Some(target) = private_owner.filter(|target| {
            self.value_type(receiver)
                != &JvmType::Object(self.cx.program.class(target.class).name.clone())
        }) {
            Expr::cast(
                self.cx
                    .types
                    .class(&self.cx.program.class(target.class).name),
                self.expression(receiver, false, depth + 1)?,
            )
        } else if declaration
            .zip(source_target)
            .is_some_and(|(declared, actual)| {
                self.cx.program.method_throws(declared) != self.cx.program.method_throws(actual)
            })
        {
            Expr::cast(
                self.cx.types.class(&method.owner),
                self.expression(receiver, false, depth + 1)?,
            )
        } else {
            self.receiver(receiver, &JvmType::Object(method.owner.clone()), depth + 1)?
        };
        Ok(qualifier.member(name).invoke(arguments))
    }

    pub fn constructor_call(
        &self,
        method: &Member,
        receiver: ValueId,
        args: &[ValueId],
        offset: u32,
    ) -> Result<Expr> {
        ensure!(
            self.proven_this(receiver, &mut BTreeSet::new()),
            "constructor delegation receiver is not this"
        );
        let descriptor = MethodDescriptor::parse(&method.descriptor)?;
        let keyword = if method.owner == self.cx.program.class(self.cx.types.owner).name {
            "this"
        } else {
            "super"
        };
        let mut arguments = self.arguments(
            args,
            &descriptor.parameters,
            &super::overloads::Site {
                owners: &[method.owner.as_str()],
                name: "<init>",
                constructor: true,
            },
            0,
        )?;
        self.class_name_arguments(offset, &mut arguments);
        let mut effects = super::exceptions::Thrown::default();
        super::exceptions::call_effects(
            self.cx,
            &method.owner,
            &method.name,
            &method.descriptor,
            InvokeKind::Special,
            &mut effects,
        );
        Ok(Expr::atom(keyword).invoke(arguments).with_throws(effects))
    }

    fn class_name_arguments(&self, offset: u32, arguments: &mut [Expr]) {
        let Some(analysis) = self.semantic_analysis() else {
            return;
        };
        for input in analysis.class_name_arguments(offset) {
            if input.translate
                && let Some(argument) = arguments.get_mut(input.parameter)
                && let Some(helper) = self.cx.class_name_helper(input.names)
            {
                let value = std::mem::replace(argument, Expr::null());
                *argument = Expr::atom(helper).invoke(vec![value]);
            }
        }
    }

    pub(super) fn class_name_field(&self, offset: u32, value: Expr) -> Expr {
        if let Some(input) = self
            .semantic_analysis()
            .and_then(|analysis| analysis.class_name_field(offset))
            && input.translate
            && let Some(helper) = self.cx.class_name_helper(input.names)
        {
            Expr::atom(helper).invoke(vec![value])
        } else {
            value
        }
    }

    fn proven_this(&self, id: ValueId, visiting: &mut BTreeSet<ValueId>) -> bool {
        if !visiting.insert(id) {
            return true;
        }
        match &self.ir.values[id].kind {
            ValueKind::Alias(value) => self.proven_this(*value, visiting),
            ValueKind::Parameter { index: None, .. } => true,
            ValueKind::Phi { inputs, .. } => {
                !inputs.is_empty()
                    && inputs
                        .iter()
                        .all(|input| self.proven_this(input.value, visiting))
            }
            _ => false,
        }
    }

    pub(super) fn condition_expression(
        &self,
        condition: &Condition,
        negated: bool,
        as_value: bool,
    ) -> Result<Expr> {
        let op = if negated {
            condition.op.inverted()
        } else {
            condition.op
        };
        let left = &self.ir.values[condition.left];
        let right = &self.ir.values[condition.right];
        if matches!(op, CompareOp::Eq | CompareOp::Ne) {
            for (value, constant) in [
                (condition.left, condition.right),
                (condition.right, condition.left),
            ] {
                if self.ir.values[constant].local.is_none()
                    && let ValueKind::Literal(Literal::Int(constant @ (0 | 1))) =
                        self.ir.values[constant].kind
                    && let Some(expression) = self.boolean_test(value, constant, op, as_value)?
                {
                    return Ok(expression);
                }
            }
        }
        if right.local.is_none()
            && matches!(right.kind, ValueKind::Literal(Literal::Int(0)))
            && let ValueKind::Compare { kind, left, right } = left.kind
            && self.ir.values[condition.left].local.is_none()
        {
            let (left, right) =
                super::conversions::binary_operands(self.ir, left, right, &compare_type(kind));
            let lhs = self.expression(left, false, 0)?;
            let rhs = self.expression(right, false, 0)?;
            // JVM cmpg/cmpl give NaN a signed result. Negating the
            // opposite relation preserves it without evaluating twice.
            let invert = matches!(kind, CompareKind::FloatLess | CompareKind::DoubleLess)
                && matches!(op, CompareOp::Lt | CompareOp::Le)
                || matches!(kind, CompareKind::FloatGreater | CompareKind::DoubleGreater)
                    && matches!(op, CompareOp::Gt | CompareOp::Ge);
            return Ok(if invert {
                Expr::prefix(
                    Prefix::Not,
                    Expr::binary(comparison(op.inverted()), lhs, rhs),
                )
            } else {
                Expr::binary(comparison(op), lhs, rhs)
            });
        }
        let left_ty = self.value_type(condition.left);
        let right_ty = self.value_type(condition.right);
        let (lhs, rhs) = if matches!(op, CompareOp::Eq | CompareOp::Ne)
            && left_ty == &JvmType::Boolean
            && right_ty == &JvmType::Boolean
        {
            // Java equality already compares these operand slots directly.
            // Converting each to 0/1 is unnecessary and loses the distinction
            // between noncanonical boolean arguments in legacy bytecode.
            (
                self.expression(condition.left, false, 0)?,
                self.expression(condition.right, false, 0)?,
            )
        } else if matches!(op, CompareOp::Eq | CompareOp::Ne)
            && left_ty.is_reference()
            && right_ty.is_reference()
            && !super::types::cast_compatible(
                self.cx.program,
                self.cx.types.names,
                left_ty,
                right_ty,
            )
        {
            (
                Expr::cast(
                    "java.lang.Object",
                    self.expression(condition.left, false, 0)?,
                ),
                self.expression(condition.right, false, 0)?,
            )
        } else if left_ty == &JvmType::Boolean && right_ty.is_int_like()
            || right_ty == &JvmType::Boolean && left_ty.is_int_like()
        {
            (
                self.coerced(condition.left, &JvmType::Int, false, 0)?,
                self.coerced(condition.right, &JvmType::Int, false, 0)?,
            )
        } else {
            (
                self.comparison_operand(condition.left, right_ty)?,
                self.comparison_operand(condition.right, left_ty)?,
            )
        };
        Ok(Expr::binary(comparison(op), lhs, rhs))
    }

    /// Keep exact-one equality distinct from zero/nonzero tests until the
    /// producer proves 0/1. A Boolean descriptor alone cannot make that proof.
    fn boolean_test(
        &self,
        value: ValueId,
        constant: i32,
        op: CompareOp,
        as_value: bool,
    ) -> Result<Option<Expr>> {
        let selected = self.ir.values[value].local.is_none()
            && matches!(self.ir.values[value].kind, ValueKind::Select { .. });
        if !selected && self.value_type(value) != &JvmType::Boolean {
            return Ok(None);
        }
        let usage = ValueUse::Reference(value);
        let canonical = self.canonical_boolean(usage);
        let truth_test = constant == 0 || canonical;
        let inverted = (op == CompareOp::Eq) == (constant == 0);
        if selected && truth_test {
            // The comparison consumes a predicate even when the selected value
            // still has an int type. Project its arms before spelling the test.
            let context = if as_value && canonical {
                super::source::Context::Operand(&JvmType::Boolean)
            } else {
                super::source::Context::Test
            };
            let expression = super::source::value(self, usage, context, inverted)?;
            return Ok(Some(if as_value && !canonical {
                expression.truth_value()
            } else {
                expression
            }));
        }
        if self.value_type(value) != &JvmType::Boolean {
            return Ok(None);
        }
        let expression = self.expression(value, false, 0)?;
        Ok(Some(if !truth_test {
            // javac preserves this as an integer-slot comparison. Converting
            // to 0/1 first would incorrectly make a raw argument 2 equal true.
            Expr::binary(comparison(op), expression, Expr::boolean(true))
        } else if inverted {
            Expr::prefix(Prefix::Not, expression)
        } else if as_value && !canonical {
            // A returned predicate produces 0/1; a direct slot transfer need
            // not. Truth contexts, including logical operands, need no wrapper.
            Expr::conditional(expression, Expr::boolean(true), Expr::boolean(false))
        } else {
            expression
        }))
    }

    fn comparison_operand(&self, value: ValueId, peer: &JvmType) -> Result<Expr> {
        if *peer == JvmType::Char
            && self.semantic_literal(value).is_none()
            && let Some(character) = literals::character_operand(self.ir, value)
        {
            return Ok(Expr::atom(character).with_type(Some(&JvmType::Char)));
        }
        self.expression(value, false, 0)
    }

    /// Plan the structured condition DAG with Java's left-to-right short
    /// circuit order. Negation distributes through the DAG without reversing it.
    pub(super) fn predicate(
        &self,
        predicate: &crate::structure::Predicate,
        negated: bool,
        as_value: bool,
    ) -> Result<Expr> {
        fn plan(
            planner: &ExprPlanner<'_, '_>,
            predicate: &crate::structure::Predicate,
            negated: bool,
            as_value: bool,
        ) -> Result<Expr> {
            use crate::structure::Predicate;
            match predicate {
                Predicate::Atom {
                    head,
                    negated: atom_negated,
                } => {
                    let crate::ir::Terminator::Branch { condition, .. } =
                        &planner.ir.blocks[*head].terminator
                    else {
                        bail!("predicate head is not a branch");
                    };
                    // A predicate leaf can render as a logical expression after
                    // selected-value recovery. Its emitted operator determines
                    // grouping, independently of its original CFG node kind.
                    let negated = negated ^ atom_negated;
                    planner.condition_expression(condition, negated, as_value)
                }
                Predicate::And(parts) | Predicate::Or(parts) => {
                    let and = matches!(predicate, Predicate::And(_)) != negated;
                    let parts = parts
                        .iter()
                        .map(|part| plan(planner, part, negated, as_value && parts.len() == 1))
                        .collect::<Result<Vec<_>>>()?;
                    Ok(Expr::logical(and, parts))
                }
                Predicate::Select { condition, yes, no } => {
                    if super::source::same_local_predicate(planner.ir, yes, no)
                        && condition.heads().into_iter().all(|head| {
                            planner.ir.blocks[head]
                                .terminator
                                .operands()
                                .into_iter()
                                .all(|value| planner.discardable(value))
                        })
                    {
                        return plan(planner, yes, negated, as_value);
                    }
                    Ok(Expr::conditional(
                        plan(planner, condition, false, false)?,
                        plan(planner, yes, negated, as_value)?,
                        plan(planner, no, negated, as_value)?,
                    ))
                }
            }
        }
        plan(self, predicate, negated, as_value)
    }

    /// A source read may be discarded only when it neither changes state nor
    /// throws. Unlike scheduling's ordering rule, this also recognizes a
    /// resolved nonvolatile field read on the exact, non-null `this` binding.
    pub(super) fn discardable(&self, value: ValueId) -> bool {
        use crate::ir::scheduled::{self, ValueUse};
        scheduled::values(self.ir, ValueUse::Reference(value))
            .operations()
            .all(|value| {
                if !self.ir.operation_is_observable(value) {
                    return true;
                }
                let ValueKind::Field {
                    field,
                    receiver: Some(receiver),
                } = &self.ir.values[value].kind
                else {
                    return false;
                };
                self.ir.is_this(*receiver)
                    && self
                        .cx
                        .program
                        .resolve_field(&field.owner, &field.name, &field.descriptor)
                        .is_some_and(|field| {
                            self.cx.program.field(field).access_flags & 0x0048 == 0
                        })
            })
    }

    fn value_type(&self, value: ValueId) -> &JvmType {
        super::conversions::source_type(self.ir, value)
    }
}

/// Nonfinite floating constants have no Java literal token. Keep their portable
/// arithmetic spelling structured too, rather than smuggling it into an atom.
pub(super) fn literal_expression(literal: &Literal) -> Expr {
    let (value, suffix) = match literal {
        Literal::Null => return Expr::null(),
        Literal::Int(value) => {
            return Expr::integer(i64::from(*value), JvmType::Int, value.to_string());
        }
        Literal::Long(value) => return Expr::integer(*value, JvmType::Long, format!("{value}L")),
        Literal::Float(value) if !value.is_finite() => (*value as f64, "F"),
        Literal::Double(value) if !value.is_finite() => (*value, "D"),
        _ => return Expr::atom(literals::primitive_literal(literal).unwrap()),
    };
    let numerator = if value.is_nan() {
        "0.0"
    } else if value.is_sign_negative() {
        "-1.0"
    } else {
        "1.0"
    };
    Expr::binary(
        Operator::Divide,
        Expr::atom(format!("{numerator}{suffix}")),
        Expr::atom(format!("0.0{suffix}")),
    )
    .grouped()
}

pub(super) fn relocated_class_name(original: &str, names: &NamePlan) -> Option<String> {
    crate::semantics::class_names::relocated(original, names)
}

fn compare_type(kind: CompareKind) -> JvmType {
    match kind {
        CompareKind::Long => JvmType::Long,
        CompareKind::FloatLess | CompareKind::FloatGreater => JvmType::Float,
        CompareKind::DoubleLess | CompareKind::DoubleGreater => JvmType::Double,
    }
}

pub(super) fn binary_operator(op: BinaryOp) -> Operator {
    match op {
        BinaryOp::Mul => Operator::Multiply,
        BinaryOp::Div => Operator::Divide,
        BinaryOp::Rem => Operator::Remainder,
        BinaryOp::Add => Operator::Add,
        BinaryOp::Sub => Operator::Subtract,
        BinaryOp::Shl => Operator::ShiftLeft,
        BinaryOp::Shr => Operator::ShiftRight,
        BinaryOp::Ushr => Operator::UnsignedShiftRight,
        BinaryOp::And => Operator::And,
        BinaryOp::Xor => Operator::Xor,
        BinaryOp::Or => Operator::Or,
    }
}
pub(super) fn comparison(op: CompareOp) -> Operator {
    match op {
        CompareOp::Eq => Operator::Equal,
        CompareOp::Ne => Operator::NotEqual,
        CompareOp::Lt => Operator::Less,
        CompareOp::Le => Operator::LessEqual,
        CompareOp::Gt => Operator::Greater,
        CompareOp::Ge => Operator::GreaterEqual,
    }
}

pub fn helper_name(program: &Program<'_>, types: &TypeNames<'_>, helper: Helper) -> String {
    support_name(program, types, helper.name())
}

pub(super) fn support_name(program: &Program<'_>, types: &TypeNames<'_>, base: &str) -> String {
    let occupied = |candidate: &str| {
        std::iter::once(types.owner)
            .chain(program.ancestors(types.owner).iter().copied())
            .any(|class| {
                program
                    .class(class)
                    .methods
                    .iter()
                    .enumerate()
                    .any(|(index, _)| {
                        types
                            .names
                            .method_name(crate::program::MemberId { class, index })
                            == candidate
                    })
            })
    };
    if !occupied(base) {
        return base.into();
    }
    let mut index = 0;
    loop {
        let name = format!("{base}_{index}");
        if !occupied(&name) {
            return name;
        }
        index += 1;
    }
}
