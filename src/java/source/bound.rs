//! Fully bound source: operands, conversions and member selection are fixed,
//! while lexical structure and local identity remain available to rewrites.
use crate::java::{
    body::default_value,
    syntax::{Expr, Operator, Syntax, syntax},
};
use crate::{classfile::JvmType, ir::LocalId};
use std::collections::BTreeMap;

pub(in crate::java) type Body = super::Body<Expr, Simple, Selector, Throw>;
pub(in crate::java) type Statement = super::Statement<Expr, Simple, Selector, Throw>;
pub(in crate::java) type LoopHeader = super::LoopHeader<Expr, Simple>;

pub(in crate::java) struct Throw {
    pub value: Expr,
    /// Throwing the value is distinct from evaluating its expression. Keep the
    /// selected rethrow contract even if its local producer is substituted.
    pub effects: crate::java::exceptions::Thrown,
}

pub(in crate::java) struct Binding {
    pub local: LocalId,
    pub name: String,
    pub ty: JvmType,
    pub type_name: String,
    pub synthetic: bool,
    pub canonical_boolean: bool,
    /// This local joins writes to a bytecode storage slot, rather than stack arms.
    pub storage: bool,
}

/// The consumer type is fixed at binding, including its selected source spelling.
/// Conditional operands need it before Java applies its own arm promotion rules.
pub(in crate::java) struct ValueType {
    pub ty: JvmType,
    pub name: String,
}

impl ValueType {
    pub fn operand(&self, value: Expr) -> Expr {
        if value.is_null() || value.source_type() == Some(&self.ty) {
            value
        } else {
            Expr::cast(&self.name, value).with_type(Some(&self.ty))
        }
    }
}

impl Binding {
    pub fn value_type(&self) -> ValueType {
        ValueType {
            ty: self.ty.clone(),
            name: self.type_name.clone(),
        }
    }

    /// A substituted initializer must retain the local's Java apparent type.
    /// This is an emitted cast, not a promise in metadata: overload resolution,
    /// conditional promotion and narrowing must see the original stored type.
    /// Contextual normalization can discharge it once the consumer is known.
    pub fn replacement(&self, value: Expr) -> Expr {
        if value.source_type() == Some(&self.ty) {
            value
        } else {
            Expr::cast(&self.type_name, value).with_type(Some(&self.ty))
        }
    }

    /// The right-hand value of a local write. Compound operations include their
    /// implicit narrowing; ordinary assignment/operand conversions remain with
    /// the consumer so null keeps its bottom type in conditional arms.
    pub fn assigned_value(&self, value: &AssignmentValue) -> Expr {
        let (operator, right) = match value {
            AssignmentValue::Value(value) => return value.clone(),
            AssignmentValue::Compound { operator, value } => (*operator, value.clone()),
            AssignmentValue::Increment { increment } => (
                if *increment {
                    Operator::Add
                } else {
                    Operator::Subtract
                },
                Expr::integer(1, JvmType::Int, "1"),
            ),
        };
        let ty = if self.ty == JvmType::Boolean {
            Some(JvmType::Boolean)
        } else if matches!(
            operator,
            Operator::ShiftLeft | Operator::ShiftRight | Operator::UnsignedShiftRight
        ) {
            Some(if self.ty == JvmType::Long {
                JvmType::Long
            } else {
                JvmType::Int
            })
        } else {
            right
                .source_type()
                .and_then(|right| crate::java::conversions::promoted(&self.ty, right))
        };
        self.replacement(Expr::binary(operator, self.expression(), right).with_type(ty.as_ref()))
    }

    pub fn expression(&self) -> Expr {
        Expr::local(self.local, &self.name, &self.ty).with_canonical_boolean(self.canonical_boolean)
    }
}

pub(in crate::java) enum AssignmentValue {
    Value(Expr),
    Compound { operator: Operator, value: Expr },
    Increment { increment: bool },
}

impl AssignmentValue {
    pub fn into_expression(self, target: Expr) -> Expr {
        match self {
            Self::Value(value) => Expr::assignment(target, value, None),
            Self::Compound { operator, value } => Expr::assignment(target, value, Some(operator)),
            Self::Increment { increment } => Expr::update(target, increment, true),
        }
    }
}

pub(in crate::java) enum Simple {
    Declare {
        binding: Binding,
        initialize: bool,
    },
    Assign {
        binding: Binding,
        value: AssignmentValue,
        declaration: bool,
    },
    Expression(Expr),
    /// Compiler-generated dispatch/discard declarations have no scheduled local.
    /// Their initializer remains a complete expression with visible effects.
    Generated {
        type_name: String,
        value: Expr,
    },
}

pub(in crate::java) struct Selector {
    pub value: Expr,
    pub keys: BTreeMap<i32, Expr>,
}

pub(in crate::java) struct Plan {
    pub lines: Vec<Line>,
    pub thrown: crate::java::exceptions::Thrown,
    indent: usize,
}
pub(in crate::java) struct Line {
    pub indent: usize,
    pub syntax: Syntax,
}

impl Plan {
    pub fn enclose(&mut self, opening: Syntax, prefix: Vec<Syntax>, suffix: Vec<(usize, Syntax)>) {
        let mut lines = Vec::with_capacity(self.lines.len() + prefix.len() + suffix.len() + 1);
        lines.push(Line {
            indent: 0,
            syntax: opening,
        });
        lines.extend(prefix.into_iter().map(|syntax| Line { indent: 1, syntax }));
        lines.extend(self.lines.drain(..).map(|mut line| {
            line.indent += 1;
            line
        }));
        lines.extend(
            suffix
                .into_iter()
                .map(|(indent, syntax)| Line { indent, syntax }),
        );
        self.lines = lines;
    }
    fn line(&mut self, syntax: impl Into<Syntax>) {
        self.lines.push(Line {
            indent: self.indent,
            syntax: syntax.into(),
        });
    }
    fn open(&mut self, syntax: impl Into<Syntax>) {
        self.line(syntax);
        self.indent += 1;
    }
    fn close(&mut self, syntax: impl Into<Syntax>) {
        self.indent -= 1;
        self.line(syntax);
    }

    fn body(&mut self, body: Body) {
        for statement in body {
            self.statement(statement);
        }
    }
    fn statement(&mut self, statement: Statement) {
        match statement {
            Statement::Simple(value) => self.line(syntax![simple(value), ";"]),
            Statement::Return(value) => self.line(match value {
                Some(value) => syntax!["return ", value, ";"],
                None => "return;".into(),
            }),
            Statement::Throw(value) => self.line(syntax!["throw ", value.value, ";"]),
            Statement::Break(label) => self.line(transfer("break", &label)),
            Statement::Continue(label) => self.line(transfer("continue", &label)),
            Statement::Block { label, body } => {
                self.open(syntax![prefix(&label), "{"]);
                self.body(body);
                self.close("}");
            }
            Statement::If { condition, yes, no } => self.conditional(condition, yes, no),
            Statement::Loop {
                label,
                header,
                body,
            } => {
                let (opening, closing) = match header {
                    LoopHeader::While(condition) => {
                        (syntax!["while (", condition, ")"], Syntax::from("}"))
                    }
                    LoopHeader::DoWhile(condition) => {
                        (Syntax::from("do"), syntax!["} while (", condition, ");"])
                    }
                    LoopHeader::For {
                        initializer,
                        condition,
                        update,
                    } => (
                        syntax![
                            "for (",
                            initializer.map(simple).unwrap_or_else(|| Syntax::from("")),
                            "; ",
                            condition,
                            "; ",
                            simple(*update),
                            ")"
                        ],
                        Syntax::from("}"),
                    ),
                };
                self.open(syntax![prefix(&label), opening, " {"]);
                self.body(body);
                self.close(closing);
            }
            Statement::Switch {
                label,
                value,
                cases,
            } => {
                self.open(syntax![prefix(&label), "switch (", value.value, ") {"]);
                let mut keys = value.keys;
                for case in cases {
                    for key in case.keys {
                        self.line(syntax![
                            "case ",
                            keys.remove(&key).expect("bound case key"),
                            ":"
                        ]);
                    }
                    if case.default {
                        self.line("default:");
                    }
                    self.indent += 1;
                    if case.scoped {
                        self.open("{");
                    }
                    self.body(case.body);
                    if case.scoped {
                        self.close("}");
                    }
                    self.indent -= 1;
                }
                self.close("}");
            }
            Statement::Try {
                body,
                catches,
                cleanup,
            } => {
                self.open("try {");
                self.body(body);
                for catch in catches {
                    self.close(syntax!["} catch (", catch.ty, " ", catch.name, ") {"]);
                    self.indent += 1;
                    self.body(catch.body);
                }
                if let Some(cleanup) = cleanup {
                    self.close("} finally {");
                    self.indent += 1;
                    self.body(cleanup);
                }
                self.close("}");
            }
            Statement::Synchronized { monitor, body } => {
                self.open(syntax!["synchronized (", monitor, ") {"]);
                self.body(body);
                self.close("}");
            }
        }
    }
    fn conditional(&mut self, condition: Expr, mut yes: Body, mut no: Body) {
        self.open(syntax!["if (", condition, ") {"]);
        loop {
            self.body(yes);
            if no.len() == 1 && matches!(no.first(), Some(Statement::If { .. })) {
                let Statement::If {
                    condition,
                    yes: nested_yes,
                    no: nested_no,
                } = no.pop().unwrap()
                else {
                    unreachable!()
                };
                self.close(syntax!["} else if (", condition, ") {"]);
                self.indent += 1;
                yes = nested_yes;
                no = nested_no;
            } else {
                if !no.is_empty() {
                    self.close("} else {");
                    self.indent += 1;
                    self.body(no);
                }
                self.close("}");
                break;
            }
        }
    }
}

pub(super) fn layout(body: Body, thrown: crate::java::exceptions::Thrown) -> Plan {
    let mut plan = Plan {
        lines: Vec::new(),
        thrown,
        indent: 0,
    };
    plan.body(body);
    plan
}

fn simple(value: Simple) -> Syntax {
    match value {
        Simple::Declare {
            binding,
            initialize,
        } => syntax![
            binding.type_name,
            " ",
            binding.name,
            if initialize {
                syntax![" = ", default_value(&binding.ty)]
            } else {
                Syntax::from("")
            }
        ],
        Simple::Assign {
            binding,
            value,
            declaration,
        } => {
            let target = binding.expression();
            syntax![
                if declaration {
                    syntax![binding.type_name, " "]
                } else {
                    Syntax::from("")
                },
                value.into_expression(target)
            ]
        }
        Simple::Expression(value) => value.into(),
        Simple::Generated { type_name, value } => syntax![type_name, " ", value],
    }
}

fn prefix(label: &Option<String>) -> String {
    label
        .as_ref()
        .map(|label| format!("{label}: "))
        .unwrap_or_default()
}
fn transfer(keyword: &str, label: &Option<String>) -> String {
    match label {
        Some(label) => format!("{keyword} {label};"),
        None => format!("{keyword};"),
    }
}
