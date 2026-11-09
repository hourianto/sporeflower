//! Fully bound source: operands, conversions and member selection are fixed,
//! while lexical structure and local identity remain available to rewrites.
use crate::java::{
    body::{CodeWriter, default_value},
    syntax::{Expr, Operator},
};
use crate::{classfile::JvmType, ir::LocalId};
use std::collections::BTreeMap;

pub(in crate::java) use super::{Body, LoopHeader, Statement};

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
    pub text: String,
    pub thrown: crate::java::exceptions::Thrown,
}

impl CodeWriter {
    fn source_body(&mut self, body: Body) {
        let mut statements = body.into_iter().peekable();
        let mut first = true;
        let mut declarations = 0;
        let mut previous_substantial = false;
        while let Some(statement) = statements.next() {
            let declaration = statement.is_declaration();
            let substantial = statement.is_substantial();
            let starts_declarations = declaration
                && declarations == 0
                && statements.peek().is_some_and(Statement::is_declaration);
            // Keep declaration runs together and attach a lone declaration to
            // the following statement. Only larger control-flow chunks need
            // their own paragraph; short guards and loops remain compact.
            if !first
                && (starts_declarations
                    || (declarations > 1 && !declaration)
                    || (previous_substantial
                        && !matches!(statement, Statement::Break(_) | Statement::Continue(_)))
                    || (substantial && declarations == 0))
            {
                self.line("");
            }
            self.statement(statement);
            declarations = if declaration { declarations + 1 } else { 0 };
            previous_substantial = substantial;
            first = false;
        }
    }
    fn statement(&mut self, statement: Statement) {
        match statement {
            Statement::Simple(value) => self.line(format!("{};", simple(value))),
            Statement::Return(value) => self.line(match value {
                Some(value) => format!("return {};", value.render()),
                None => "return;".into(),
            }),
            Statement::Throw(value) => self.line(format!("throw {};", value.value.render())),
            Statement::Break(label) => self.line(transfer("break", &label)),
            Statement::Continue(label) => self.line(transfer("continue", &label)),
            Statement::Block { label, body } => {
                self.open(format!("{}{{", prefix(&label)));
                self.source_body(body);
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
                        (format!("while ({})", condition.render()), String::from("}"))
                    }
                    LoopHeader::DoWhile(condition) => (
                        String::from("do"),
                        format!("}} while ({});", condition.render()),
                    ),
                    LoopHeader::For {
                        initializer,
                        condition,
                        update,
                    } => (
                        format!(
                            "for ({}; {}; {})",
                            initializer.map(|value| simple(*value)).unwrap_or_default(),
                            condition.render(),
                            simple(*update)
                        ),
                        String::from("}"),
                    ),
                };
                self.open(format!("{}{opening} {{", prefix(&label)));
                self.source_body(body);
                self.close(closing);
            }
            Statement::Switch {
                label,
                value,
                cases,
            } => {
                self.open(format!(
                    "{}switch ({}) {{",
                    prefix(&label),
                    value.value.render()
                ));
                let mut keys = value.keys;
                for case in cases {
                    for key in case.keys {
                        self.line(format!(
                            "case {}:",
                            keys.remove(&key).expect("bound case key").render()
                        ));
                    }
                    if case.default {
                        self.line("default:");
                    }
                    self.indent += 1;
                    if case.scoped {
                        self.open("{");
                    }
                    self.source_body(case.body);
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
                self.source_body(body);
                for catch in catches {
                    self.close(format!("}} catch ({} {}) {{", catch.ty, catch.name));
                    self.indent += 1;
                    self.source_body(catch.body);
                }
                if let Some(cleanup) = cleanup {
                    self.close("} finally {");
                    self.indent += 1;
                    self.source_body(cleanup);
                }
                self.close("}");
            }
            Statement::Synchronized { monitor, body } => {
                self.open(format!("synchronized ({}) {{", monitor.render()));
                self.source_body(body);
                self.close("}");
            }
        }
    }
    fn conditional(&mut self, condition: Expr, mut yes: Body, mut no: Body) {
        self.open(format!("if ({}) {{", condition.render()));
        loop {
            self.source_body(yes);
            if no.len() == 1 && matches!(no.first(), Some(Statement::If { .. })) {
                let Statement::If {
                    condition,
                    yes: nested_yes,
                    no: nested_no,
                } = no.pop().unwrap()
                else {
                    unreachable!()
                };
                self.close(format!("}} else if ({}) {{", condition.render()));
                self.indent += 1;
                yes = nested_yes;
                no = nested_no;
            } else {
                if !no.is_empty() {
                    self.close("} else {");
                    self.indent += 1;
                    self.source_body(no);
                }
                self.close("}");
                break;
            }
        }
    }
}

impl Statement {
    fn is_declaration(&self) -> bool {
        matches!(
            self,
            Self::Simple(
                Simple::Declare { .. }
                    | Simple::Assign {
                        declaration: true,
                        ..
                    }
                    | Simple::Generated { .. }
            )
        )
    }

    fn is_substantial(&self) -> bool {
        match self {
            Self::If { yes, no, .. } => !compact(yes) || !compact(no),
            Self::Loop { body, .. }
            | Self::Block { body, .. }
            | Self::Synchronized { body, .. } => !compact(body),
            Self::Switch { .. } | Self::Try { .. } => true,
            _ => false,
        }
    }
}

fn compact(body: &[Statement]) -> bool {
    matches!(
        body,
        [] | [Statement::Simple(_)
            | Statement::Return(_)
            | Statement::Throw(_)
            | Statement::Break(_)
            | Statement::Continue(_)]
    )
}

pub(super) fn layout(body: Body, thrown: crate::java::exceptions::Thrown) -> Plan {
    let mut out = CodeWriter::new(0);
    out.source_body(body);
    Plan {
        text: out.text,
        thrown,
    }
}

fn simple(value: Simple) -> String {
    match value {
        Simple::Declare {
            binding,
            initialize,
        } => {
            let mut text = format!("{} {}", binding.type_name, binding.name);
            if initialize {
                text.push_str(" = ");
                text.push_str(default_value(&binding.ty));
            }
            text
        }
        Simple::Assign {
            binding,
            value,
            declaration,
        } => {
            let expression = value.into_expression(binding.expression()).render();
            if declaration {
                format!("{} {expression}", binding.type_name)
            } else {
                expression
            }
        }
        Simple::Expression(value) => value.render(),
        Simple::Generated { type_name, value } => format!("{type_name} {}", value.render()),
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

#[cfg(test)]
#[path = "../../../tests/unit/source_layout.rs"]
mod tests;
