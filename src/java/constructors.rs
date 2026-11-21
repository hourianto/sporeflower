//! Constructor delegation is a graph boundary, not the first textual statement.
//!
//! Java requires delegation before declarations. Pure copies and unobservable
//! receiver-field initialization can cross that boundary. A preinitialization
//! helper returns one typed carrier for statements or branches before delegation.
//! The carrier transports both arguments and locals live in the remaining body.
use super::body::CodeWriter;
use super::context::{BodyKind, BodyOptions, EmissionContext};
use super::generated::{Carrier, Slot};
use super::local_flow::{live_at_entry, statement_reads, value_reads};
use super::source::ConstructorEntry;
use super::syntax::Expr;
use super::types::TypeNames;
use crate::Result;
use crate::classfile::{JvmType, MethodDescriptor};
use crate::hash::{HashMap, HashSet};
use crate::index::{BlockId, IdVec, LocalId};
use crate::ir::{Local, Member, MethodIr, Statement, Terminator, ValueId, ValueKind, ValueUse};
use crate::program::{MemberId, Program};
use crate::structure::PreparedIr;
use anyhow::{Context, ensure};
use std::collections::BTreeSet;
use std::fmt::Write as _;

mod carrier;
mod defaults;
mod delegation;

pub(super) fn normalize_finals(
    cx: &EmissionContext<'_, '_>,
    member: MemberId,
    ir: &mut MethodIr,
) -> Result<()> {
    delegation::lower(cx, member, ir)?;
    defaults::complete(cx.program, member.class, ir);
    Ok(())
}

/// Only original assignments to this object's instance finals need special
/// treatment when inlining this(...) or filling a path's default value.
fn final_store<'a>(
    program: &Program<'_>,
    owner: usize,
    ir: &MethodIr,
    statement: &'a Statement,
) -> Option<(MemberId, &'a Member, ValueId)> {
    let Statement::PutField {
        field,
        receiver: Some(receiver),
        ..
    } = statement
    else {
        return None;
    };
    let id = program.resolve_field(&field.owner, &field.name, &field.descriptor)?;
    (id.class == owner
        && ir.aliases_this(*receiver)
        && program.field(id).access_flags & (crate::classfile::STATIC | crate::classfile::FINAL)
            == crate::classfile::FINAL
        && program.initialization_plan().permits_final(id))
    .then_some((id, field, *receiver))
}

struct Delegation {
    block: BlockId,
    statement: usize,
    target: Member,
    receiver: ValueId,
    args: Vec<ValueId>,
    pc: u32,
}

impl Delegation {
    fn reachable(ir: &MethodIr) -> impl Iterator<Item = Self> + '_ {
        ir.reachable_blocks().into_iter().flat_map(|block| {
            ir.blocks[block]
                .statements
                .iter()
                .enumerate()
                .filter_map(move |(statement, action)| {
                    let Statement::ConstructorCall {
                        method,
                        receiver,
                        args,
                        pc,
                    } = action
                    else {
                        return None;
                    };
                    Some(Self {
                        block,
                        statement,
                        target: method.clone(),
                        receiver: *receiver,
                        args: args.clone(),
                        pc: *pc,
                    })
                })
        })
    }
}

/// Source-only delegation choices are selected before binding type names. A
/// constructor absent from the bytecode, or an unreadable constructor body,
/// can require parameter and exception types absent from the constant pool.
pub(super) struct GeneratedDelegations {
    absent: Option<DefaultDelegation>,
    failures: HashMap<usize, Result<DefaultDelegation>>,
}

struct DefaultDelegation {
    marker: bool,
    target: Option<MemberId>,
    exceptions: Vec<String>,
}

impl DefaultDelegation {
    fn arguments(&self, program: &Program<'_>, types: &TypeNames<'_>) -> Result<String> {
        if self.marker {
            Ok("false".into())
        } else if let Some(target) = self.target {
            default_arguments(&program.method(target).descriptor, types)
        } else {
            Ok(String::new())
        }
    }
}

impl GeneratedDelegations {
    pub fn build(program: &Program<'_>, owner: usize) -> Result<Self> {
        let class = program.class(owner);
        let failures: HashMap<_, _> = class
            .methods
            .iter()
            .enumerate()
            .filter(|(_, method)| method.name == "<init>")
            .map(|(index, _)| {
                let allowed = program.method_throws(MemberId {
                    class: owner,
                    index,
                });
                (index, absent_delegation(program, owner, Some(allowed)))
            })
            .collect();
        let absent = if class.access_flags & 0x0200 == 0 && failures.is_empty() {
            Some(absent_delegation(program, owner, None)?)
        } else {
            None
        };
        Ok(Self { absent, failures })
    }

    pub fn types(&self, program: &Program<'_>) -> Result<Vec<JvmType>> {
        let mut types = Vec::new();
        for delegation in self.absent.iter().chain(
            self.failures
                .values()
                .filter_map(|choice| choice.as_ref().ok()),
        ) {
            if let Some(target) = delegation.target {
                types.extend(
                    MethodDescriptor::parse(&program.method(target).descriptor)?.parameters,
                );
            }
            types.extend(delegation.exceptions.iter().cloned().map(JvmType::Object));
        }
        Ok(types)
    }
}

/// A classfile may have no constructor at all. A source declaration must still
/// suppress javac's implicit no-argument constructor. The marker parameter keeps
/// Class.newInstance failing exactly as before, even inside the same package.
pub(super) fn absent(
    program: &Program<'_>,
    types: &TypeNames<'_>,
    generated: &GeneratedDelegations,
    out: &mut CodeWriter,
) -> Result<()> {
    let owner = types.owner;
    let class = program.class(owner);
    let delegation = generated
        .absent
        .as_ref()
        .expect("planned absent constructor");
    let arguments = delegation.arguments(program, types)?;
    let has_subclass = program
        .classes()
        .iter()
        .take(program.own_count())
        .any(|subclass| subclass.super_name.as_deref() == Some(&class.name));
    let simple = types.simple_name();
    let mut header = format!(
        "{} {simple}(boolean $unused)",
        if has_subclass { "protected" } else { "private" }
    );
    header.push_str(&super::generated::throws_clause(
        &delegation.exceptions,
        types,
    ));
    out.line("");
    out.open(format!("{header} {{"));
    if !arguments.is_empty() {
        out.line(format!("super({arguments});"));
    }
    out.line("throw new java.lang.Error();");
    out.close("}");
    Ok(())
}

fn absent_delegation(
    program: &Program<'_>,
    owner: usize,
    allowed: Option<&[String]>,
) -> Result<DefaultDelegation> {
    let class = program.class(owner);
    let Some(parent) = class
        .super_name
        .as_deref()
        .and_then(|name| program.id(name))
    else {
        return Ok(DefaultDelegation {
            marker: false,
            target: None,
            exceptions: Vec::new(),
        });
    };
    let constructors: Vec<_> = program
        .class(parent)
        .methods
        .iter()
        .enumerate()
        .filter(|(_, method)| method.name == "<init>")
        .collect();
    if constructors.is_empty() && program.is_own(parent) {
        return Ok(DefaultDelegation {
            marker: true,
            target: None,
            exceptions: absent_delegation(program, parent, allowed)?.exceptions,
        });
    }
    if constructors.is_empty() {
        return Ok(DefaultDelegation {
            marker: false,
            target: None,
            exceptions: Vec::new(),
        });
    }
    let (index, _) = constructors
        .into_iter()
        .filter(|(index, method)| {
            program.visible_from(method.access_flags, parent, owner)
                && allowed.is_none_or(|allowed| {
                    compatible_throws(
                        program,
                        MemberId {
                            class: parent,
                            index: *index,
                        },
                        allowed,
                    )
                })
        })
        .map(|(index, method)| {
            Ok((
                index,
                MethodDescriptor::parse(&method.descriptor)?
                    .parameters
                    .len(),
            ))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .min_by_key(|(_, arity)| *arity)
        .with_context(|| {
            format!(
                "{} has no source-accessible constructor with compatible throws in {}",
                class.name,
                program.class(parent).name
            )
        })?;
    let target = MemberId {
        class: parent,
        index,
    };
    Ok(DefaultDelegation {
        marker: false,
        target: Some(target),
        exceptions: program.method_throws(target).to_vec(),
    })
}

fn default_arguments(descriptor: &str, types: &TypeNames<'_>) -> Result<String> {
    Ok(MethodDescriptor::parse(descriptor)?
        .parameters
        .iter()
        .map(|ty| match ty {
            JvmType::Byte
            | JvmType::Short
            | JvmType::Char
            | JvmType::Object(_)
            | JvmType::Array(_) => format!("({}) {}", types.ty(ty), super::body::default_value(ty)),
            _ => super::body::default_value(ty).to_owned(),
        })
        .collect::<Vec<_>>()
        .join(", "))
}

fn compatible_throws(program: &Program<'_>, target: MemberId, allowed: &[String]) -> bool {
    program.method_throws(target).iter().all(|exception| {
        !crate::throws::is_checked(program, exception)
            || allowed
                .iter()
                .any(|allowed| program.is_subtype(exception, allowed))
    })
}

/// Even an explicit failure body needs a legal first constructor invocation.
/// Prefer the original super delegation; when lifting itself failed, use an
/// accessible superclass constructor without widening the throws declaration.
/// Reusing this(...) could reproduce a rejected recursive-constructor cycle.
fn failure_delegation(
    program: &Program<'_>,
    types: &TypeNames<'_>,
    generated: &GeneratedDelegations,
    member: MemberId,
) -> Result<String> {
    let class = program.class(member.class);
    let allowed = program.method_throws(member);
    if let Ok(ir) = program.original_method_ir(member) {
        for block in ir.reachable_blocks() {
            for statement in &ir.blocks[block].statements {
                if let Statement::ConstructorCall {
                    method, receiver, ..
                } = statement
                    && ir.aliases_this(*receiver)
                    && class.super_name.as_deref() == Some(method.owner.as_str())
                    && program.id(&method.owner).is_none_or(|owner| {
                        program
                            .declared_method(owner, "<init>", &method.descriptor)
                            .is_some_and(|target| {
                                program.visible_from(
                                    program.method(target).access_flags,
                                    owner,
                                    member.class,
                                ) && compatible_throws(program, target, allowed)
                            })
                    })
                {
                    return Ok(format!(
                        "super({});",
                        default_arguments(&method.descriptor, types)?
                    ));
                }
            }
        }
    }
    let delegation = generated.failures[&member.index]
        .as_ref()
        .map_err(|error| anyhow::anyhow!("{error:#}"))?;
    let arguments = delegation.arguments(program, types)?;
    Ok(format!("super({arguments});"))
}

pub(super) fn emit_failure(
    cx: &EmissionContext<'_, '_>,
    generated: &GeneratedDelegations,
    member: MemberId,
    header: &str,
    diagnostic: &str,
    out: &mut CodeWriter,
) -> Result<()> {
    let program = cx.program;
    let types = cx.types;
    let delegation = failure_delegation(program, types, generated, member)?;
    out.open(format!("{header} {{"));
    let parent = program.class(member.class).super_name.as_deref();
    if parent.is_none()
        || parent == Some("java/lang/Object")
            && program.can_use_platform_contract("java/lang/Object")
    {
        out.line(delegation);
        out.line(format!("throw new java.lang.Error({diagnostic});"));
        out.close("}");
        return Ok(());
    }

    // A real superclass constructor can call an override on the partial
    // receiver or throw its own exception. Fail during argument evaluation,
    // before even a no-arg superclass constructor starts. A recovered inner
    // class places its static marker in its enclosing support container.
    let marker = cx.constructor_failure_marker(
        helper_name(program, types, member.index, "$ConstructorFailure"),
        diagnostic,
    );
    let (parameters, arguments) = bridge_markers(program, types)?;
    out.line(format!("this(new {marker}(){arguments});"));
    out.close("}");
    out.line("");
    let throws = super::generated::throws_clause(program.method_throws(member), types);
    out.open(format!(
        "private {}({marker} $failure{parameters}){throws} {{",
        types.simple_name()
    ));
    out.line(delegation);
    out.line("throw new java.lang.Error();");
    out.close("}");
    Ok(())
}

pub(super) fn render(
    cx: &EmissionContext<'_, '_>,
    prepared: PreparedIr,
    member: MemberId,
    default_fields: &[Expr],
) -> Result<super::RenderedMethod> {
    let program = cx.program;
    let types = cx.types;
    let ir = prepared.ir();
    let mut calls: Vec<_> = Delegation::reachable(ir).collect();
    ensure!(
        calls.iter().all(|call| ir.aliases_this(call.receiver)),
        "delegation receiver is not the uninitialized constructor receiver"
    );
    if delegation::shared_implicit_super(
        ir,
        program.class(types.owner).super_name.as_deref(),
        &calls,
    ) {
        return render_direct(
            cx,
            &prepared,
            member,
            ConstructorEntry {
                invocation: None,
                crossed: Vec::new(),
                defaults: default_fields,
                delegates: false,
            },
        );
    }
    ensure!(
        calls.len() == 1,
        "constructor has {} reachable delegation sites; expected a common initialization boundary",
        calls.len()
    );
    let call = calls.pop().unwrap();
    ensure!(
        ir.blocks[call.block].handlers.is_empty(),
        "constructor delegation is covered by an exception handler, which Java source cannot enclose"
    );

    // A normal constructor often begins in a synthetic phi-copy entry block.
    // Follow that chain and inline only nonthrowing, state-independent copies in
    // the delegation expression. Their body assignments may safely follow it.
    if let Some(crossed) = direct_prefix(cx, ir, &call) {
        let delegates = call.target.owner == program.class(types.owner).name;
        let invocation = (delegates || call.target.descriptor != "()V")
            .then(|| &ir.blocks[call.block].statements[call.statement]);
        if let Ok(body) = cx.artifact(|candidate| {
            render_direct(
                candidate,
                &prepared,
                member,
                ConstructorEntry {
                    invocation,
                    crossed,
                    defaults: default_fields,
                    delegates,
                },
            )
        }) {
            return Ok(cx.commit(body));
        }
    }

    carrier::render(cx, prepared.into_ir(), member, call, default_fields)
}

fn render_direct(
    cx: &EmissionContext<'_, '_>,
    prepared: &PreparedIr,
    member: MemberId,
    entry: ConstructorEntry<'_>,
) -> Result<super::RenderedMethod> {
    let rendered = cx.render(
        prepared,
        Some(member),
        BodyOptions::original(BodyKind::Constructor(entry)),
    )?;
    Ok(super::RenderedMethod {
        body: rendered.text,
        declarations: String::new(),
        fallback: rendered.fallback,
        initializers: Vec::new(),
    })
}

fn direct_prefix<'a>(
    cx: &EmissionContext<'_, '_>,
    ir: &'a MethodIr,
    call: &Delegation,
) -> Option<Vec<&'a Statement>> {
    let mut crossed = Vec::new();
    let mut block = ir.entry;
    let mut seen = HashSet::default();
    let mut stores = Vec::new();
    loop {
        if !seen.insert(block) || !ir.blocks[block].handlers.is_empty() {
            return None;
        }
        let body = &ir.blocks[block];
        let prefix = if block == call.block {
            &body.statements[..call.statement]
        } else {
            &body.statements
        };
        for statement in prefix {
            if !matches!(statement, Statement::Assign { .. }) {
                stores.push(
                    cx.program
                        .plain_receiver_store(cx.types.owner, ir, statement)?,
                );
            }
            crossed.push(statement);
        }
        if block == call.block {
            return (stores.is_empty()
                || cx.program.delegation_preserves_receiver(
                    cx.types.owner,
                    &call.target,
                    &stores,
                ))
            .then_some(crossed);
        }
        let Terminator::Goto(next) = body.terminator else {
            return None;
        };
        block = next;
    }
}

fn prune(ir: &mut MethodIr) -> BTreeSet<BlockId> {
    let seen = ir.reachable_blocks();
    for (id, block) in ir.blocks.iter_enumerated_mut() {
        if !seen.contains(&id) {
            block.statements.clear();
            block.terminator = Terminator::Unreachable;
            block.handlers.clear();
        }
    }
    seen
}

fn helper_name(program: &Program<'_>, types: &TypeNames<'_>, index: usize, prefix: &str) -> String {
    super::generated::symbol(program, types, &format!("{prefix}{index}"))
}

fn bridge_markers(program: &Program<'_>, types: &TypeNames<'_>) -> Result<(String, String)> {
    let hidden = usize::from(types.units.capture(types.owner).is_some());
    let arities: BTreeSet<_> = program
        .class(types.owner)
        .methods
        .iter()
        .filter(|method| method.name == "<init>")
        .map(|method| {
            MethodDescriptor::parse(&method.descriptor)
                .map(|descriptor| descriptor.parameters.len() - hidden)
        })
        .collect::<Result<_>>()?;
    let arity = (1..255 - hidden)
        .find(|arity| !arities.contains(arity))
        .context("no unused constructor arity for a private bridge")?;
    let mut parameters = String::new();
    for index in 1..arity {
        write!(parameters, ", boolean $marker{index}").unwrap();
    }
    Ok((parameters, ", false".repeat(arity - 1)))
}
