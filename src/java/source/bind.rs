//! Bind the finalized source scopes once. Every conversion, member selection,
//! update and switch choice is complete before the layout plan can be printed.
use super::{Body, Expression, LoopHeader, Selector, Simple, Statement, Throw};
use crate::java::{
    body::default_value,
    expr::ExprPlanner,
    syntax::{Expr, Syntax, syntax},
};
use crate::{
    Result,
    classfile::JvmType,
    ir::{Statement as Scheduled, ValueKind},
};
use anyhow::ensure;

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
}

pub(super) fn bind(
    expr: &ExprPlanner<'_, '_>,
    body: &Body,
    thrown: crate::java::exceptions::Thrown,
) -> Result<Plan> {
    let mut binder = Binder {
        expr,
        out: Plan {
            lines: Vec::new(),
            thrown,
            indent: 0,
        },
    };
    binder.body(body)?;
    Ok(binder.out)
}

struct Binder<'a, 'b, 'c> {
    expr: &'c ExprPlanner<'a, 'b>,
    out: Plan,
}

impl Binder<'_, '_, '_> {
    pub fn body(&mut self, body: &Body) -> Result<()> {
        for statement in body {
            self.statement(statement)?;
        }
        Ok(())
    }

    fn statement(&mut self, statement: &Statement) -> Result<()> {
        match statement {
            Statement::Simple(simple) => self.out.line(syntax![self.simple(simple)?, ";"]),
            Statement::Return(value) => self.out.line(match value {
                Some(value) => syntax![
                    "return ",
                    value.plan(self.expr, &self.expr.ir.return_type)?,
                    ";"
                ],
                None => "return;".into(),
            }),
            Statement::Throw(value) => self.out.line(syntax!["throw ", self.throw(value)?, ";"]),
            Statement::Break(label) => self.out.line(transfer("break", label)),
            Statement::Continue(label) => self.out.line(transfer("continue", label)),
            Statement::Block { label, body } => {
                self.out.open(syntax![prefix(label), "{"]);
                self.body(body)?;
                self.out.close("}");
            }
            Statement::If { condition, yes, no } => self.conditional(condition, yes, no)?,
            Statement::Loop {
                label,
                header,
                body,
            } => {
                let opening = match header {
                    LoopHeader::While(condition) => {
                        syntax!["while (", condition.condition(self.expr)?, ")"]
                    }
                    LoopHeader::DoWhile(_) => "do".into(),
                    LoopHeader::For {
                        initializer,
                        condition,
                        update,
                    } => syntax![
                        "for (",
                        initializer
                            .as_ref()
                            .map(|initializer| self.simple(initializer))
                            .transpose()?
                            .unwrap_or_else(|| Syntax::from("")),
                        "; ",
                        condition.condition(self.expr)?,
                        "; ",
                        self.simple(update)?,
                        ")"
                    ],
                };
                self.out.open(syntax![prefix(label), opening, " {"]);
                self.body(body)?;
                self.out.close(match header {
                    LoopHeader::DoWhile(condition) => {
                        syntax!["} while (", condition.condition(self.expr)?, ");"]
                    }
                    _ => "}".into(),
                });
            }
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
                        let text = expression;
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
                            Expr::cast("int", text.grouped())
                        } else {
                            text
                        };
                        (value, character)
                    }
                    Selector::Dispatch(value) => (Expr::atom(value), false),
                };
                self.out
                    .open(syntax![prefix(label), "switch (", value, ") {"]);
                for case in cases {
                    for key in &case.keys {
                        let key = if let Some(expression) =
                            selector.and_then(|selector| self.expr.switch_key(selector, *key))
                        {
                            expression
                        } else if character && let Ok(key) = u16::try_from(*key) {
                            Expr::atom(crate::java::literals::char_literal(key))
                        } else {
                            Expr::atom(key.to_string())
                        };
                        self.out.line(syntax!["case ", key, ":"]);
                    }
                    if case.default {
                        self.out.line("default:");
                    }
                    self.out.indent += 1;
                    if case.scoped {
                        self.out.open("{");
                    }
                    self.body(&case.body)?;
                    if case.scoped {
                        self.out.close("}");
                    }
                    self.out.indent -= 1;
                }
                self.out.close("}");
            }
            Statement::Try {
                body,
                catches,
                cleanup,
            } => {
                self.out.open("try {");
                self.body(body)?;
                for catch in catches {
                    self.out
                        .close(syntax!["} catch (", &catch.ty, " ", &catch.name, ") {"]);
                    self.out.indent += 1;
                    self.body(&catch.body)?;
                }
                if let Some(cleanup) = cleanup {
                    self.out.close("} finally {");
                    self.out.indent += 1;
                    self.body(cleanup)?;
                }
                self.out.close("}");
            }
            Statement::Synchronized { monitor, body } => {
                self.out.open(syntax![
                    "synchronized (",
                    monitor.plan(self.expr, &JvmType::Object("java/lang/Object".into()))?,
                    ") {"
                ]);
                self.body(body)?;
                self.out.close("}");
            }
        }
        Ok(())
    }

    fn simple(&self, statement: &Simple) -> Result<Syntax> {
        Ok(match statement {
            Simple::Scheduled(statement) => self.scheduled(statement)?,
            Simple::CatchBind { local, name, ty } => Expr::assignment(
                Expr::atom(self.expr.local_name(*local)),
                match ty {
                    Some(ty) => Expr::cast(ty, Expr::atom(name)),
                    None => Expr::atom(name),
                },
                None,
            )
            .into(),
            Simple::Dispatch {
                state,
                target,
                declare,
            } => syntax![
                if *declare { "int " } else { "" },
                state,
                " = ",
                target.to_string()
            ],
            Simple::Declare { local, initialize } => {
                let binding = &self.expr.ir.locals[*local];
                syntax![
                    self.expr.cx.types.ty(&binding.ty),
                    " ",
                    self.expr.local_name(*local),
                    if *initialize {
                        syntax![" = ", default_value(&binding.ty)]
                    } else {
                        Syntax::from("")
                    }
                ]
            }
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
                    return Ok(update);
                }
                let binding = &self.expr.ir.locals[*local];
                let prefix = if *declaration {
                    syntax![self.expr.cx.types.ty(&binding.ty), " "]
                } else {
                    Syntax::from("")
                };
                let value = value.plan(self.expr, &binding.ty)?;
                syntax![
                    prefix,
                    Expr::assignment(Expr::atom(self.expr.local_name(*local)), value, None,)
                ]
            }
        })
    }

    fn scheduled(&self, statement: &Scheduled) -> Result<Syntax> {
        use crate::java::source_updates;
        Ok(match statement {
            Scheduled::Assign { local, value } => self.simple(&Simple::Assign {
                local: *local,
                value: Expression::value(self.expr.ir, *value, &self.expr.ir.locals[*local].ty),
                declaration: false,
            })?,
            Scheduled::Eval(value) => {
                let expression = self.expr.statement_root(*value)?;
                match self.expr.ir.values[*value].kind {
                    ValueKind::Call { .. }
                    | ValueKind::New { .. }
                    | ValueKind::InitializeClass { .. }
                    | ValueKind::Update { .. }
                    | ValueKind::CompoundUpdate { .. }
                    | ValueKind::LocalUpdate { .. } => expression.into(),
                    _ => syntax![
                        self.expr.cx.types.ty(&self.expr.ir.values[*value].ty),
                        " $discard",
                        value.to_string(),
                        " = ",
                        expression
                    ],
                }
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
                        .into()
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
                    .into()
                }
            }
            Scheduled::ConstructorCall {
                method,
                receiver,
                args,
                pc,
            } => self
                .expr
                .constructor_call(method, *receiver, args, *pc)?
                .into(),
            Scheduled::Monitor { .. } => {
                unreachable!("source lowering rejects unstructured monitors")
            }
        })
    }

    fn throw(&self, throwing: &Throw) -> Result<Expr> {
        let (value, pc) = match throwing {
            Throw::Witness(ty) => return Ok(Expr::cast(ty, Expr::atom("null"))),
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

    fn conditional<'a>(
        &mut self,
        condition: &Expression,
        mut yes: &'a Body,
        mut no: &'a Body,
    ) -> Result<()> {
        self.out
            .open(syntax!["if (", condition.condition(self.expr)?, ") {"]);
        loop {
            self.body(yes)?;
            if let [
                Statement::If {
                    condition,
                    yes: nested_yes,
                    no: nested_no,
                },
            ] = no.as_slice()
            {
                self.out.close(syntax![
                    "} else if (",
                    condition.condition(self.expr)?,
                    ") {"
                ]);
                self.out.indent += 1;
                yes = nested_yes;
                no = nested_no;
            } else {
                if !no.is_empty() {
                    self.out.close("} else {");
                    self.out.indent += 1;
                    self.body(no)?;
                }
                self.out.close("}");
                break;
            }
        }
        Ok(())
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
