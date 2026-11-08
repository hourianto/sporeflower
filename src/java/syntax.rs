//! Bound Java expressions retain local identity and apparent Java types,
//! without bytecode or name-resolution context.
//! Planning selects operands, conversions and bindings; spelling owns grouping
//! and layout. Expressions cannot be assembled from arbitrary syntax fragments,
//! and their precedence is derived from their form rather than supplied by callers.
//! There is deliberately no `Display`: interpolation cannot silently render a
//! child while semantic planning is still in progress.
use crate::{classfile::JvmType, ir::LocalId};

#[derive(Clone, Debug)]
pub(super) struct Expr {
    pub kind: Expression,
    pub facts: ExpressionFacts,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ExpressionFacts {
    pub ty: Option<JvmType>,
    pub canonical_boolean: bool,
    pub integer_literal: Option<i64>,
    /// Sparse operation facts keep ordinary expression nodes compact.
    pub details: Option<Box<ExpressionDetails>>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ExpressionDetails {
    /// Preparation identities are consumed and cleared before source rewrites.
    pub origin: Option<SourceLeaf>,
    pub publication: Option<(crate::program::MemberId, bool)>,
    /// Intrinsic contract only; descendants retain their own effects.
    pub throws: super::exceptions::Thrown,
}

/// Stable preparation leaves; a literal ID denotes one immutable definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum SourceLeaf {
    Local(LocalId),
    Literal(crate::ir::ValueId),
}

#[derive(Clone, Debug)]
pub(super) enum Expression {
    Atom(String),
    Null,
    Integer {
        value: i64,
        ty: JvmType,
        text: String,
    },
    Local {
        id: LocalId,
        name: String,
        ty: JvmType,
    },
    /// A selected field reference, including unqualified static fields.
    Field(Box<Expr>),
    Boolean(bool),
    Commented(Box<Expr>, String),
    Group(Box<Expr>),
    Prefix(Prefix, Box<Expr>),
    Update {
        target: Box<Expr>,
        increment: bool,
        postfix: bool,
    },
    Binary(Operator, Box<[Expr; 2]>),
    Logical {
        and: bool,
        parts: Vec<Expr>,
    },
    Cast {
        ty: String,
        primitive: bool,
        value: Box<Expr>,
    },
    InstanceOf {
        value: Box<Expr>,
        ty: String,
    },
    Member {
        receiver: Box<Expr>,
        name: String,
    },
    Index(Box<[Expr; 2]>),
    Invocation {
        target: Box<Expr>,
        arguments: Vec<Expr>,
    },
    New {
        class: String,
        arguments: Vec<Expr>,
    },
    NewArray {
        element: String,
        dimensions: Vec<Expr>,
        trailing: usize,
    },
    Assignment {
        operands: Box<[Expr; 2]>,
        compound: Option<Operator>,
    },
    Conditional(Box<[Expr; 3]>),
    Concatenation(Vec<Expr>),
    ArrayInitializer {
        head: String,
        elements: Vec<Expr>,
        nested: bool,
    },
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Prefix {
    Negate,
    Not,
    Complement,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Operator {
    Multiply,
    Divide,
    Remainder,
    Add,
    Subtract,
    ShiftLeft,
    ShiftRight,
    UnsignedShiftRight,
    And,
    Xor,
    Or,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

impl Operator {
    fn text(self) -> &'static str {
        match self {
            Self::Multiply => "*",
            Self::Divide => "/",
            Self::Remainder => "%",
            Self::Add => "+",
            Self::Subtract => "-",
            Self::ShiftLeft => "<<",
            Self::ShiftRight => ">>",
            Self::UnsignedShiftRight => ">>>",
            Self::And => "&",
            Self::Xor => "^",
            Self::Or => "|",
            Self::Equal => "==",
            Self::NotEqual => "!=",
            Self::Less => "<",
            Self::LessEqual => "<=",
            Self::Greater => ">",
            Self::GreaterEqual => ">=",
        }
    }

    fn precedence(self) -> u8 {
        match self {
            Self::Multiply | Self::Divide | Self::Remainder => 13,
            Self::Add | Self::Subtract => 12,
            Self::ShiftLeft | Self::ShiftRight | Self::UnsignedShiftRight => 11,
            Self::Less | Self::LessEqual | Self::Greater | Self::GreaterEqual => 10,
            Self::Equal | Self::NotEqual => 9,
            Self::And => 8,
            Self::Xor => 7,
            Self::Or => 6,
        }
    }

    fn comparison(self) -> bool {
        matches!(
            self,
            Self::Equal
                | Self::NotEqual
                | Self::Less
                | Self::LessEqual
                | Self::Greater
                | Self::GreaterEqual
        )
    }
}

impl Expr {
    fn node(expression: Expression) -> Self {
        let ty = match &expression {
            Expression::Boolean(_) | Expression::InstanceOf { .. } | Expression::Logical { .. } => {
                Some(JvmType::Boolean)
            }
            Expression::Local { ty, .. } | Expression::Integer { ty, .. } => Some(ty.clone()),
            Expression::Binary(op, _) if op.comparison() => Some(JvmType::Boolean),
            Expression::Prefix(Prefix::Not, _) => Some(JvmType::Boolean),
            Expression::Conditional(parts) if parts[1].facts.ty == parts[2].facts.ty => {
                parts[1].facts.ty.clone()
            }
            Expression::Conditional(parts) if parts[1].is_null() => parts[2]
                .facts
                .ty
                .as_ref()
                .filter(|ty| ty.is_reference())
                .cloned(),
            Expression::Conditional(parts) if parts[2].is_null() => parts[1]
                .facts
                .ty
                .as_ref()
                .filter(|ty| ty.is_reference())
                .cloned(),
            Expression::Group(value)
            | Expression::Commented(value, _)
            | Expression::Field(value) => value.facts.ty.clone(),
            Expression::Update { target, .. } => target.facts.ty.clone(),
            Expression::Assignment { operands, .. } => operands[0].facts.ty.clone(),
            _ => None,
        };
        let canonical = match &expression {
            Expression::Boolean(_)
            | Expression::InstanceOf { .. }
            | Expression::Prefix(Prefix::Not, _) => true,
            Expression::Binary(operator, _) if operator.comparison() => true,
            Expression::Binary(Operator::And, operands) => {
                operands.iter().any(Self::canonical_boolean)
            }
            Expression::Binary(Operator::Or | Operator::Xor, operands) => {
                operands.iter().all(Self::canonical_boolean)
            }
            Expression::Logical { parts, .. } => {
                parts.len() > 1 || parts.first().is_some_and(Self::canonical_boolean)
            }
            Expression::Conditional(parts) => {
                parts[1].canonical_boolean() && parts[2].canonical_boolean()
            }
            Expression::Group(value)
            | Expression::Commented(value, _)
            | Expression::Field(value)
            | Expression::Cast { value, .. } => value.canonical_boolean(),
            Expression::Assignment {
                operands,
                compound: None,
            } => operands[1].canonical_boolean(),
            Expression::Assignment {
                operands,
                compound: Some(Operator::And),
            } => operands.iter().any(Self::canonical_boolean),
            Expression::Assignment {
                operands,
                compound: Some(Operator::Or | Operator::Xor),
            } => operands.iter().all(Self::canonical_boolean),
            _ => false,
        };
        let integer = match &expression {
            Expression::Integer { value, .. } => Some(*value),
            Expression::Group(value) | Expression::Commented(value, _) => value.integer_value(),
            _ => None,
        };
        Self {
            kind: expression,
            facts: ExpressionFacts {
                ty,
                canonical_boolean: canonical,
                integer_literal: integer,
                details: None,
            },
        }
    }

    /// Positive evidence that this Boolean expression produces exactly 0/1.
    /// A Boolean descriptor alone deliberately supplies no such evidence.
    pub fn canonical_boolean(&self) -> bool {
        self.facts.canonical_boolean
    }

    pub fn with_canonical_boolean(mut self, proven: bool) -> Self {
        self.facts.canonical_boolean |= proven;
        self
    }

    /// A known integral literal keeps its semantic value even when binding
    /// chooses a symbolic constant spelling. Arithmetic presentation may still
    /// retain that spelling rather than treating it as an unnamed literal.
    pub fn integer_value(&self) -> Option<i64> {
        self.facts.integer_literal
    }

    pub fn with_integer_value(mut self, value: i64) -> Self {
        self.facts.integer_literal = Some(value);
        self
    }

    pub fn null() -> Self {
        Self::node(Expression::Null)
    }

    /// The null bottom type belongs to expressions, never to stored references.
    /// A cast of null has the named cast type and is intentionally excluded.
    pub fn is_null(&self) -> bool {
        match &self.kind {
            Expression::Null => true,
            Expression::Group(value) | Expression::Commented(value, _) => value.is_null(),
            Expression::Conditional(parts) => parts[1].is_null() && parts[2].is_null(),
            _ => false,
        }
    }

    pub fn integer(value: i64, ty: JvmType, text: impl Into<String>) -> Self {
        Self::node(Expression::Integer {
            value,
            ty,
            text: text.into(),
        })
    }

    pub fn local(id: LocalId, name: impl Into<String>, ty: &JvmType) -> Self {
        Self::node(Expression::Local {
            id,
            name: name.into(),
            ty: ty.clone(),
        })
    }

    pub fn field(self) -> Self {
        Self::node(Expression::Field(Box::new(self)))
    }

    pub fn with_type(mut self, ty: Option<&JvmType>) -> Self {
        self.facts.ty = ty.cloned();
        self
    }

    pub fn with_throws(mut self, effects: super::exceptions::Thrown) -> Self {
        if !effects.is_empty() {
            self.facts
                .details
                .get_or_insert_with(Default::default)
                .throws
                .extend(effects);
        }
        self
    }

    pub fn source_type(&self) -> Option<&JvmType> {
        self.facts.ty.as_ref()
    }

    /// A previously bound name, type name or literal, never a composed expression.
    pub fn atom(text: impl Into<String>) -> Self {
        Self::node(Expression::Atom(text.into()))
    }

    pub fn boolean(value: bool) -> Self {
        Self::node(Expression::Boolean(value))
    }

    pub fn boolean_constant(&self) -> Option<bool> {
        match &self.kind {
            Expression::Boolean(value) => Some(*value),
            _ => None,
        }
    }

    /// Whether an already-bound member reference uses only names. A static
    /// constant accessed through a primary expression loses Java constantness.
    /// This tests syntax after binding, not whether an arbitrary atom is a name.
    pub fn is_named_reference(&self) -> bool {
        match &self.kind {
            Expression::Atom(_) | Expression::Local { .. } => true,
            Expression::Field(value) => value.is_named_reference(),
            Expression::Member { receiver, .. } => receiver.is_named_reference(),
            _ => false,
        }
    }

    pub fn grouped(self) -> Self {
        Self::node(Expression::Group(Box::new(self)))
    }

    pub fn commented(self, comment: String) -> Self {
        Self::node(Expression::Commented(Box::new(self), comment))
    }

    pub fn prefix(op: Prefix, value: Self) -> Self {
        if matches!(op, Prefix::Not) {
            value.negated(false)
        } else {
            Self::node(Expression::Prefix(op, Box::new(value)))
        }
    }

    /// Negation of a control-flow test observes truth, not raw Boolean bits.
    /// General prefix construction deliberately retains double negation when
    /// it materializes a canonical 0/1 value from a legacy Boolean producer.
    pub fn negated_test(self) -> Self {
        self.negated(true)
    }

    fn negated(self, test: bool) -> Self {
        match self.kind {
            Expression::Boolean(value) => Self::boolean(!value),
            Expression::Prefix(Prefix::Not, value) if test || value.canonical_boolean() => *value,
            Expression::Logical { and, mut parts } => match parts.len() {
                0 => Self::boolean(!and),
                1 => parts.pop().unwrap().negated(test),
                _ => Self::logical(!and, parts.into_iter().map(Self::negated_test).collect()),
            },
            Expression::Group(value) => value.negated(test).grouped(),
            Expression::Commented(value, comment) => value.negated(test).commented(comment),
            Expression::Binary(comparison, operands)
                if comparison.comparison()
                    && (matches!(comparison, Operator::Equal | Operator::NotEqual)
                        || operands.iter().all(|operand| {
                            matches!(
                                operand.source_type(),
                                Some(
                                    JvmType::Byte
                                        | JvmType::Short
                                        | JvmType::Char
                                        | JvmType::Int
                                        | JvmType::Long
                                )
                            )
                        })) =>
            {
                // Equality inversion preserves exact raw Boolean bits. Ordered
                // comparisons invert only when their Java types exclude NaN.
                let inverse = match comparison {
                    Operator::Equal => Operator::NotEqual,
                    Operator::NotEqual => Operator::Equal,
                    Operator::Less => Operator::GreaterEqual,
                    Operator::LessEqual => Operator::Greater,
                    Operator::Greater => Operator::LessEqual,
                    Operator::GreaterEqual => Operator::Less,
                    _ => unreachable!(),
                };
                Self::node(Expression::Binary(inverse, operands))
            }
            _ => Self::node(Expression::Prefix(Prefix::Not, Box::new(self))),
        }
    }

    pub fn update(target: Self, increment: bool, postfix: bool) -> Self {
        Self::node(Expression::Update {
            target: Box::new(target),
            increment,
            postfix,
        })
    }

    pub fn binary(op: Operator, left: Self, right: Self) -> Self {
        Self::node(Expression::Binary(op, Box::new([left, right])))
    }

    pub fn logical(and: bool, parts: Vec<Self>) -> Self {
        Self::node(Expression::Logical { and, parts })
    }

    pub fn cast(ty: impl Into<String>, value: Self) -> Self {
        let ty = ty.into();
        // Java's cast grammar distinguishes primitive keywords from reference
        // type names, even though both have the same operator precedence.
        let actual = match ty.as_str() {
            "boolean" => Some(JvmType::Boolean),
            "byte" => Some(JvmType::Byte),
            "char" => Some(JvmType::Char),
            "short" => Some(JvmType::Short),
            "int" => Some(JvmType::Int),
            "long" => Some(JvmType::Long),
            "float" => Some(JvmType::Float),
            "double" => Some(JvmType::Double),
            _ => None,
        };
        Self::node(Expression::Cast {
            ty,
            primitive: actual.is_some(),
            value: Box::new(value),
        })
        .with_type(actual.as_ref())
    }

    pub fn instance_of(value: Self, ty: impl Into<String>) -> Self {
        Self::node(Expression::InstanceOf {
            value: Box::new(value),
            ty: ty.into(),
        })
    }

    pub fn member(self, name: impl Into<String>) -> Self {
        Self::node(Expression::Member {
            receiver: Box::new(self),
            name: name.into(),
        })
    }

    pub fn index(self, index: Self) -> Self {
        Self::node(Expression::Index(Box::new([self, index])))
    }

    pub fn invoke(self, arguments: Vec<Self>) -> Self {
        Self::node(Expression::Invocation {
            target: Box::new(self),
            arguments,
        })
    }

    pub fn new(class: impl Into<String>, arguments: Vec<Self>) -> Self {
        Self::node(Expression::New {
            class: class.into(),
            arguments,
        })
    }

    pub fn new_array(element: String, dimensions: Vec<Self>, trailing: usize) -> Self {
        Self::node(Expression::NewArray {
            element,
            dimensions,
            trailing,
        })
    }

    pub fn assignment(target: Self, value: Self, compound: Option<Operator>) -> Self {
        Self::node(Expression::Assignment {
            operands: Box::new([target, value]),
            compound,
        })
    }

    pub fn conditional(condition: Self, yes: Self, no: Self) -> Self {
        // Genuine choices read naturally with a positive selector. Bind each
        // arm before swapping so its contextual type and symbolic names remain
        // attached to the same value. Preserve an inequality test that
        // materializes 0/1 instead of spelling `test == 0 ? 0 : 1`.
        let indicator = matches!(
            (&yes.kind, &no.kind),
            (Expression::Boolean(true), Expression::Boolean(false))
        ) || matches!(
            (&yes.kind, &no.kind),
            (
                Expression::Integer { value: 1, .. },
                Expression::Integer { value: 0, .. }
            )
        ) || matches!((&yes.kind, &no.kind),
            (Expression::Atom(yes), Expression::Atom(no))
                if (yes.as_str(), no.as_str()) == ("1", "0"));
        // A simple guard followed by further alternatives is flatter than
        // moving that entire decision tree into the first arm for polarity.
        if !matches!(&no.kind, Expression::Conditional(_)) {
            match condition.kind {
                Expression::Prefix(Prefix::Not, condition) => {
                    return Self::node(Expression::Conditional(Box::new([*condition, no, yes])));
                }
                Expression::Binary(Operator::NotEqual, operands)
                    if !indicator && !operands.iter().any(Self::is_null) =>
                {
                    return Self::node(Expression::Conditional(Box::new([
                        Self::node(Expression::Binary(Operator::Equal, operands)),
                        no,
                        yes,
                    ])));
                }
                _ => {}
            }
        }
        Self::node(Expression::Conditional(Box::new([condition, yes, no])))
    }

    /// A conditional arm replacing a truth test must produce 0/1 even when
    /// legacy boolean slots or calls carry raw ints. Keep one evaluation.
    pub fn truth_value(self) -> Self {
        if self.canonical_boolean() {
            self
        } else {
            Self::prefix(Prefix::Not, Self::prefix(Prefix::Not, self))
        }
    }

    pub fn concatenation(parts: Vec<Self>) -> Self {
        Self::node(Expression::Concatenation(parts))
    }

    pub fn array_initializer(head: String, elements: Vec<Self>, nested: bool) -> Self {
        Self::node(Expression::ArrayInitializer {
            head,
            elements,
            nested,
        })
    }

    fn precedence(&self) -> u8 {
        match &self.kind {
            Expression::Commented(value, _) | Expression::Field(value) => value.precedence(),
            Expression::Assignment { .. } => 1,
            Expression::Conditional(_) => 2,
            Expression::Logical { and: false, .. } => 3,
            Expression::Logical { and: true, .. } => 4,
            Expression::Binary(op, _) => op.precedence(),
            Expression::InstanceOf { .. } => 10,
            Expression::Concatenation(_) => 12,
            Expression::Prefix(..)
            | Expression::Cast { .. }
            | Expression::Update { postfix: false, .. }
            // Another bracket would parse as an allocation dimension, so an
            // allocated array needs grouping when used as an index receiver.
            | Expression::NewArray { .. }
            | Expression::ArrayInitializer { .. } => 14,
            Expression::Update { postfix: true, .. } => 15,
            _ => 16,
        }
    }

    pub fn render(&self) -> String {
        self.render_at(0)
    }

    fn signed_leaf(&self) -> bool {
        match &self.kind {
            Expression::Atom(text) | Expression::Integer { text, .. } => {
                text.starts_with(['+', '-'])
            }
            Expression::Commented(value, _) => value.signed_leaf(),
            _ => false,
        }
    }

    fn render_at(&self, parent: u8) -> String {
        let mut output = String::new();
        self.write_at(&mut output, parent);
        output
    }

    fn write_at(&self, output: &mut String, parent: u8) {
        let grouped = self.precedence() < parent;
        if grouped {
            output.push('(');
        }
        self.write(output);
        if grouped {
            output.push(')');
        }
    }

    fn write(&self, output: &mut String) {
        match &self.kind {
            Expression::Atom(text) | Expression::Integer { text, .. } => output.push_str(text),
            Expression::Local { name, .. } => output.push_str(name),
            Expression::Field(value) => value.write(output),
            Expression::Null => output.push_str("null"),
            Expression::Boolean(value) => output.push_str(if *value { "true" } else { "false" }),
            Expression::Commented(value, comment) => {
                value.write(output);
                output.push_str(" /* ");
                output.push_str(comment);
                output.push_str(" */");
            }
            Expression::Group(value) => {
                output.push('(');
                value.write(output);
                output.push(')');
            }
            Expression::Prefix(op, value) => {
                output.push(match op {
                    Prefix::Negate => '-',
                    Prefix::Not => '!',
                    Prefix::Complement => '~',
                });
                // A signed literal is one planned leaf, but `--1` would become
                // a decrement token. Inspect that leaf, not rendered Java text.
                let signed = matches!(op, Prefix::Negate) && value.signed_leaf();
                if signed {
                    output.push('(');
                    value.write(output);
                    output.push(')');
                } else {
                    value.write_at(output, if matches!(op, Prefix::Negate) { 15 } else { 14 });
                }
            }
            Expression::Update {
                target,
                increment,
                postfix,
            } => {
                let operator = if *increment { "++" } else { "--" };
                if !postfix {
                    output.push_str(operator);
                }
                target.write_at(output, 15);
                if *postfix {
                    output.push_str(operator);
                }
            }
            Expression::Binary(op, operands) => {
                let precedence = op.precedence();
                // Keep comparison operands explicit; arithmetic preserves its
                // tree even for apparently associative floating-point operators.
                operands[0].write_at(output, if op.comparison() { 11 } else { precedence });
                output.push(' ');
                output.push_str(op.text());
                output.push(' ');
                operands[1].write_at(output, if op.comparison() { 11 } else { precedence + 1 });
            }
            Expression::Logical { and, parts } => {
                for (index, part) in parts.iter().enumerate() {
                    if index != 0 {
                        output.push_str(if *and { " && " } else { " || " });
                    }
                    part.write_at(output, if *and { 4 } else { 3 });
                }
            }
            Expression::Cast {
                ty,
                primitive,
                value,
            } => {
                output.push('(');
                output.push_str(ty);
                output.push_str(") ");
                // A reference cast takes UnaryExpressionNotPlusMinus. Numeric
                // precedence alone would print `(T) -x` as a subtraction or
                // permit an illegal prefix increment/decrement operand.
                let needs_group = !primitive
                    && match &value.kind {
                        Expression::Atom(_)
                        | Expression::Integer { .. }
                        | Expression::Commented(_, _) => value.signed_leaf(),
                        Expression::Prefix(Prefix::Negate, _) => true,
                        Expression::Update { postfix: false, .. } => true,
                        _ => false,
                    };
                if needs_group {
                    output.push('(');
                    value.write(output);
                    output.push(')');
                } else {
                    value.write_at(output, 14);
                }
            }
            Expression::InstanceOf { value, ty } => {
                value.write_at(output, 11);
                output.push_str(" instanceof ");
                output.push_str(ty);
            }
            Expression::Member { receiver, name } => {
                receiver.write_at(output, 15);
                output.push('.');
                output.push_str(name);
            }
            Expression::Index(operands) => {
                operands[0].write_at(output, 15);
                output.push('[');
                operands[1].write(output);
                output.push(']');
            }
            Expression::Invocation { target, arguments } => {
                target.write_at(output, 15);
                Self::write_arguments(arguments, output);
            }
            Expression::New { class, arguments } => {
                output.push_str("new ");
                output.push_str(class);
                Self::write_arguments(arguments, output);
            }
            Expression::NewArray {
                element,
                dimensions,
                trailing,
            } => {
                output.push_str("new ");
                output.push_str(element);
                for dimension in dimensions {
                    output.push('[');
                    dimension.write(output);
                    output.push(']');
                }
                for _ in 0..*trailing {
                    output.push_str("[]");
                }
            }
            Expression::Assignment { operands, compound } => {
                operands[0].write_at(output, 2);
                output.push(' ');
                if let Some(op) = compound {
                    output.push_str(op.text());
                }
                output.push_str("= ");
                operands[1].write_at(output, 1);
            }
            Expression::Conditional(arms) => output.push_str(&super::layout::conditional(
                arms[0].render_at(3),
                arms[1].render_at(1),
                arms[2].render_at(2),
            )),
            Expression::Concatenation(parts) => {
                let parts = parts
                    .iter()
                    .enumerate()
                    .map(|(index, part)| part.render_at(if index == 0 { 12 } else { 13 }))
                    .collect::<Vec<_>>();
                let separator = if parts.iter().map(String::len).sum::<usize>()
                    + parts.len().saturating_sub(1) * 3
                    > 100
                {
                    "\n    + "
                } else {
                    " + "
                };
                output.push_str(&parts.join(separator));
            }
            Expression::ArrayInitializer {
                head,
                elements,
                nested,
            } => {
                output.push_str(&super::array_literals::layout(
                    head,
                    &elements.iter().map(Self::render).collect::<Vec<_>>(),
                    *nested,
                ));
            }
        }
    }

    fn write_arguments(arguments: &[Self], output: &mut String) {
        output.push('(');
        output.push_str(&super::layout::arguments(
            arguments.iter().map(Self::render).collect(),
        ));
        output.push(')');
    }
}

#[cfg(test)]
#[path = "../../tests/unit/java_syntax.rs"]
mod tests;
