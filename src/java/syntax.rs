//! Bound Java syntax. These nodes contain no bytecode or name-resolution context.
//! Planning selects operands, conversions and bindings; spelling owns grouping
//! and layout. Expressions cannot be assembled from arbitrary syntax fragments,
//! and their precedence is derived from their form rather than supplied by callers.
//! There is deliberately no `Display`: interpolation cannot silently render a
//! child while semantic planning is still in progress.
use std::borrow::Cow;

#[derive(Debug)]
pub(super) enum Syntax {
    Token(Cow<'static, str>),
    Sequence(Vec<Syntax>),
    Expression(Box<Expr>),
}

#[derive(Debug)]
pub(super) struct Expr(Expression);

#[derive(Debug)]
enum Expression {
    Atom(String),
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
    /// A previously bound name, type name or literal, never a composed expression.
    pub fn atom(text: impl Into<String>) -> Self {
        Self(Expression::Atom(text.into()))
    }

    /// Whether an already-bound member reference uses only names. A static
    /// constant accessed through a primary expression loses Java constantness.
    /// This tests syntax after binding, not whether an arbitrary atom is a name.
    pub fn is_named_reference(&self) -> bool {
        match &self.0 {
            Expression::Atom(_) => true,
            Expression::Member { receiver, .. } => receiver.is_named_reference(),
            _ => false,
        }
    }

    pub fn grouped(self) -> Self {
        Self(Expression::Group(Box::new(self)))
    }

    pub fn commented(self, comment: String) -> Self {
        Self(Expression::Commented(Box::new(self), comment))
    }

    pub fn prefix(op: Prefix, value: Self) -> Self {
        if matches!(op, Prefix::Not)
            && let Expression::Binary(comparison @ (Operator::Equal | Operator::NotEqual), operands) =
                value.0
        {
            return Self(Expression::Binary(
                if matches!(comparison, Operator::Equal) {
                    Operator::NotEqual
                } else {
                    Operator::Equal
                },
                operands,
            ));
        }
        Self(Expression::Prefix(op, Box::new(value)))
    }

    pub fn update(target: Self, increment: bool, postfix: bool) -> Self {
        Self(Expression::Update {
            target: Box::new(target),
            increment,
            postfix,
        })
    }

    pub fn binary(op: Operator, left: Self, right: Self) -> Self {
        Self(Expression::Binary(op, Box::new([left, right])))
    }

    pub fn logical(and: bool, parts: Vec<Self>) -> Self {
        Self(Expression::Logical { and, parts })
    }

    pub fn cast(ty: impl Into<String>, value: Self) -> Self {
        let ty = ty.into();
        // Java's cast grammar distinguishes primitive keywords from reference
        // type names, even though both have the same operator precedence.
        let primitive = matches!(
            ty.as_str(),
            "boolean" | "byte" | "char" | "short" | "int" | "long" | "float" | "double"
        );
        Self(Expression::Cast {
            ty,
            primitive,
            value: Box::new(value),
        })
    }

    pub fn instance_of(value: Self, ty: impl Into<String>) -> Self {
        Self(Expression::InstanceOf {
            value: Box::new(value),
            ty: ty.into(),
        })
    }

    pub fn member(self, name: impl Into<String>) -> Self {
        Self(Expression::Member {
            receiver: Box::new(self),
            name: name.into(),
        })
    }

    pub fn index(self, index: Self) -> Self {
        Self(Expression::Index(Box::new([self, index])))
    }

    pub fn invoke(self, arguments: Vec<Self>) -> Self {
        Self(Expression::Invocation {
            target: Box::new(self),
            arguments,
        })
    }

    pub fn new(class: impl Into<String>, arguments: Vec<Self>) -> Self {
        Self(Expression::New {
            class: class.into(),
            arguments,
        })
    }

    pub fn new_array(element: String, dimensions: Vec<Self>, trailing: usize) -> Self {
        Self(Expression::NewArray {
            element,
            dimensions,
            trailing,
        })
    }

    pub fn assignment(target: Self, value: Self, compound: Option<Operator>) -> Self {
        Self(Expression::Assignment {
            operands: Box::new([target, value]),
            compound,
        })
    }

    pub fn conditional(condition: Self, yes: Self, no: Self) -> Self {
        Self(Expression::Conditional(Box::new([condition, yes, no])))
    }

    /// A conditional arm replacing a truth test must produce 0/1 even when
    /// legacy boolean slots or calls carry raw ints. Keep one evaluation.
    pub fn truth_value(self) -> Self {
        fn normalized(value: &Expr) -> bool {
            match &value.0 {
                Expression::Atom(text) => matches!(text.as_str(), "true" | "false"),
                Expression::Prefix(Prefix::Not, _) | Expression::InstanceOf { .. } => true,
                Expression::Group(value) | Expression::Commented(value, _) => normalized(value),
                Expression::Logical { parts, .. } => {
                    parts.len() > 1 || parts.first().is_some_and(normalized)
                }
                Expression::Binary(op, operands) => {
                    op.comparison()
                        || match op {
                            Operator::And => operands.iter().any(normalized),
                            Operator::Or | Operator::Xor => operands.iter().all(normalized),
                            _ => false,
                        }
                }
                Expression::Conditional(arms) => normalized(&arms[1]) && normalized(&arms[2]),
                _ => false,
            }
        }
        if normalized(&self) {
            self
        } else {
            Self::prefix(Prefix::Not, Self::prefix(Prefix::Not, self))
        }
    }

    pub fn concatenation(parts: Vec<Self>) -> Self {
        Self(Expression::Concatenation(parts))
    }

    pub fn array_initializer(head: String, elements: Vec<Self>, nested: bool) -> Self {
        Self(Expression::ArrayInitializer {
            head,
            elements,
            nested,
        })
    }

    fn precedence(&self) -> u8 {
        match &self.0 {
            Expression::Commented(value, _) => value.precedence(),
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
        match &self.0 {
            Expression::Atom(text) => text.starts_with(['+', '-']),
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
        match &self.0 {
            Expression::Atom(text) => output.push_str(text),
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
                    && match &value.0 {
                        Expression::Atom(_) | Expression::Commented(_, _) => value.signed_leaf(),
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

impl Syntax {
    pub fn token(text: &'static str) -> Self {
        Self::Token(Cow::Borrowed(text))
    }
    pub fn sequence(parts: Vec<Self>) -> Self {
        Self::Sequence(parts)
    }
    pub fn render(&self) -> String {
        let mut output = String::new();
        self.write(&mut output);
        output
    }
    fn write(&self, output: &mut String) {
        match self {
            Self::Token(text) => output.push_str(text),
            Self::Sequence(parts) => {
                for part in parts {
                    part.write(output);
                }
            }
            Self::Expression(expression) => expression.write(output),
        }
    }
}

impl From<Expr> for Syntax {
    fn from(expression: Expr) -> Self {
        Self::Expression(Box::new(expression))
    }
}
impl From<String> for Syntax {
    fn from(text: String) -> Self {
        Self::Token(Cow::Owned(text))
    }
}
impl From<&str> for Syntax {
    fn from(text: &str) -> Self {
        Self::Token(Cow::Owned(text.to_owned()))
    }
}
impl From<&String> for Syntax {
    fn from(text: &String) -> Self {
        Self::Token(Cow::Owned(text.clone()))
    }
}
impl From<i32> for Syntax {
    fn from(value: i32) -> Self {
        Self::Token(Cow::Owned(value.to_string()))
    }
}
impl From<usize> for Syntax {
    fn from(value: usize) -> Self {
        Self::Token(Cow::Owned(value.to_string()))
    }
}

macro_rules! syntax {
    (@parts $parts:ident;) => {};
    (@parts $parts:ident; $part:literal $(, $($rest:tt)*)?) => {
        $parts.push($crate::java::syntax::Syntax::token($part));
        $($crate::java::syntax::syntax!(@parts $parts; $($rest)*);)?
    };
    (@parts $parts:ident; $part:expr $(, $($rest:tt)*)?) => {
        $parts.push($crate::java::syntax::Syntax::from($part));
        $($crate::java::syntax::syntax!(@parts $parts; $($rest)*);)?
    };
    ($($parts:tt)*) => {{
        let mut parts = Vec::new();
        $crate::java::syntax::syntax!(@parts parts; $($parts)*);
        $crate::java::syntax::Syntax::sequence(parts)
    }};
}
pub(super) use syntax;

#[cfg(test)]
#[path = "../../tests/unit/java_syntax.rs"]
mod tests;
