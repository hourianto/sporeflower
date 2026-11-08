//! Constructor delegation is a graph boundary, not the first textual statement.
//!
//! Java requires delegation before declarations. Straight-line pure copies can
//! cross that boundary; effects cannot. A preinitialization helper returns one
//! typed carrier when the JVM performs statements or branches before delegation.
//! The carrier transports both arguments and locals live in the remaining body.
use super::body::CodeWriter;
use super::context::{BodyKind, BodyOptions, EmissionContext};
use super::local_flow::{live_at_entry, statement_reads, value_reads};
use super::types::TypeNames;
use crate::Result;
use crate::classfile::{JvmType, Method, MethodDescriptor};
use crate::index::{BlockId, IdVec, LocalId};
use crate::ir::{
    Local, Member, MethodIr, Statement, Terminator, Value, ValueId, ValueKind, ValueUse,
};
use crate::program::{MemberId, Program};
use crate::structure::PreparedIr;
use anyhow::{Context, ensure};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;

mod defaults;
mod delegation;

pub(super) fn normalize_finals(
    program: &Program<'_>,
    member: MemberId,
    ir: &mut MethodIr,
) -> Result<()> {
    delegation::lower(program, member, ir)?;
    defaults::complete(program, member.class, ir);
    Ok(())
}

struct Delegation {
    block: BlockId,
    statement: usize,
    target: Member,
    receiver: ValueId,
    args: Vec<ValueId>,
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
    let arguments = if delegation.marker {
        "false".into()
    } else if let Some(target) = delegation.target {
        default_arguments(&program.method(target).descriptor, types)?
    } else {
        String::new()
    };
    let has_subclass = program
        .classes()
        .iter()
        .take(program.own_count())
        .any(|subclass| subclass.super_name.as_deref() == Some(&class.name));
    let simple = types.names.class_name(owner).rsplit('/').next().unwrap();
    let mut header = format!(
        "{} {simple}(boolean $unused) ",
        if has_subclass { "protected" } else { "private" }
    );
    if !delegation.exceptions.is_empty() {
        header.push_str("throws ");
        header.push_str(
            &delegation
                .exceptions
                .iter()
                .map(|name| types.class(name))
                .collect::<Vec<_>>()
                .join(", "),
        );
        header.push(' ');
    }
    out.line("");
    out.open(format!("{header}{{"));
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
pub(super) fn failure_delegation(
    program: &Program<'_>,
    types: &TypeNames<'_>,
    generated: &GeneratedDelegations,
    member: MemberId,
) -> Result<String> {
    let class = program.class(member.class);
    let allowed = program.method_throws(member);
    if let Ok(ir) = crate::ir::decompile_method(class, program.method(member)) {
        for block in reachable(&ir, ir.entry, None) {
            for statement in &ir.blocks[block].statements {
                if let Statement::ConstructorCall {
                    method, receiver, ..
                } = statement
                    && is_this(&ir, *receiver, &mut HashSet::new())
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
    let arguments = if delegation.marker {
        "false".into()
    } else if let Some(target) = delegation.target {
        default_arguments(&program.method(target).descriptor, types)?
    } else {
        String::new()
    };
    Ok(format!("super({arguments});"))
}

pub(super) fn render(
    cx: &EmissionContext<'_, '_>,
    method: &Method,
    prepared: &PreparedIr,
    method_index: usize,
    default_fields: &[String],
) -> Result<super::RenderedMethod> {
    let program = cx.program;
    let types = cx.types;
    let ir = prepared.ir();
    let member = MemberId {
        class: types.owner,
        index: method_index,
    };
    let reachable = reachable(ir, ir.entry, None);
    let mut calls = Vec::new();
    for &block in &reachable {
        for (statement, action) in ir.blocks[block].statements.iter().enumerate() {
            if let Statement::ConstructorCall {
                method,
                receiver,
                args,
                ..
            } = action
            {
                ensure!(
                    is_this(ir, *receiver, &mut HashSet::new()),
                    "delegation receiver is not the uninitialized constructor receiver"
                );
                calls.push(Delegation {
                    block,
                    statement,
                    target: method.clone(),
                    receiver: *receiver,
                    args: args.clone(),
                });
            }
        }
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
    let direct = direct_arguments(ir, &call).and_then(|inline| {
        // Inlining is a source candidate: it may exceed the expression planner's
        // bound after removing scheduled storage. Retain those boundaries by
        // selecting the carrier path when the candidate cannot bind.
        cx.artifact(|candidate| {
            if call.target.owner != program.class(types.owner).name
                && call.target.descriptor == "()V"
            {
                return Ok(None);
            }
            let Statement::ConstructorCall { pc, .. } =
                ir.blocks[call.block].statements[call.statement]
            else {
                unreachable!()
            };
            Ok(Some(
                candidate
                    .expressions(&inline, Some(member))
                    .constructor_call(&call.target, call.receiver, &call.args, pc)?
                    .render(),
            ))
        })
        .ok()
    });
    if let Some(direct) = direct {
        let mut body = CodeWriter::new(2);
        if let Some(delegation) = cx.commit(direct) {
            body.line(format!("{delegation};"));
        }
        if call.target.owner != program.class(types.owner).name {
            for assignment in default_fields {
                body.line(assignment);
            }
        }
        let rendered = cx.render(
            prepared,
            Some(member),
            BodyOptions::original(BodyKind::Constructor {
                delegation: Some((call.block, call.statement)),
                delegates: call.target.owner == program.class(types.owner).name,
            }),
        )?;
        body.text.push_str(&rendered.text);
        return Ok(super::RenderedMethod {
            body: body.text,
            declarations: String::new(),
            fallback: rendered.fallback,
        });
    }

    let mut pre = ir.clone();
    pre.blocks[call.block].statements.truncate(call.statement);
    pre.blocks[call.block].terminator = Terminator::Return(None);
    pre.blocks[call.block].handlers.clear();
    let pre_reachable = prune(&mut pre);
    for &block in &pre_reachable {
        for statement in &pre.blocks[block].statements {
            for local in statement_reads(&pre, statement) {
                ensure!(
                    !pre.locals[local].is_this,
                    "preinitialization accesses uninitialized this; moving it after delegation would change superclass-observable state"
                );
            }
        }
        for value in pre.blocks[block].terminator.operands() {
            for local in value_reads(&pre, ValueUse::Reference(value)) {
                ensure!(
                    !pre.locals[local].is_this,
                    "preinitialization branch accesses uninitialized this"
                );
            }
        }
        if block != call.block {
            ensure!(
                !matches!(pre.blocks[block].terminator, Terminator::Return(_)),
                "constructor returns before initializing its receiver"
            );
        }
    }

    let mut post = ir.clone();
    post.entry = call.block;
    post.blocks[call.block].statements.drain(..=call.statement);
    let post_reachable = prune(&mut post);
    ensure!(
        pre_reachable
            .intersection(&post_reachable)
            .all(|&block| block == call.block),
        "constructor control flow crosses its initialization boundary more than once"
    );
    let captures: Vec<LocalId> = live_at_entry(&post)?
        .into_iter()
        .filter(|&local| !post.locals[local].is_this)
        .collect();
    for &argument in &call.args {
        ensure!(
            !value_reads(&pre, ValueUse::Reference(argument))
                .iter()
                .any(|&local| pre.locals[local].is_this),
            "constructor argument accesses uninitialized this"
        );
    }
    let signature = MethodDescriptor::parse(&method.descriptor)?;
    let target_signature = MethodDescriptor::parse(&call.target.descriptor)?;
    ensure!(
        target_signature.parameters.len() == call.args.len(),
        "constructor argument count mismatch"
    );
    let carrier = helper_name(program, types, method_index, "$Preinit");
    let prepare = helper_name(program, types, method_index, "$preinit");
    let carrier_type = JvmType::Object(carrier.clone());

    // Carrier reads are plain field accesses on a fresh non-null object. The
    // static helper computes the original argument expressions in JVM order.
    let state_local = pre.locals.next_id();
    pre.locals.push(Local {
        name_hint: None,
        name: "$state".into(),
        ty: carrier_type.clone(),
        parameter: None,
        is_this: false,
        needs_default: false,
    });
    let state_value = add_value(
        &mut pre,
        ValueKind::New {
            class: carrier.clone(),
            descriptor: "()V".into(),
            args: vec![],
            allocation_pc: 0,
        },
        carrier_type.clone(),
        call.block,
        Some(state_local),
    );
    pre.blocks[call.block].statements.push(Statement::Assign {
        local: state_local,
        value: crate::ir::ValueUse::Definition(state_value),
    });
    for (index, (&value, ty)) in call
        .args
        .iter()
        .zip(&target_signature.parameters)
        .enumerate()
    {
        pre.blocks[call.block].statements.push(Statement::PutField {
            pc: None,
            field: carrier_field(&carrier, format!("a{index}"), ty),
            receiver: Some(state_value),
            value,
        });
    }
    for &local in &captures {
        let value =
            local_value(&pre, local).context("live constructor local has no value identity")?;
        let field = carrier_field(&carrier, format!("v{local}"), &pre.locals[local].ty);
        pre.blocks[call.block].statements.push(Statement::PutField {
            pc: None,
            field,
            receiver: Some(state_value),
            value,
        });
    }
    pre.blocks[call.block].terminator = Terminator::Return(Some(state_value));
    pre.return_type = carrier_type.clone();

    // Original parameters become ordinary locals in the private constructor;
    // only parameters used after delegation are transported and declared.
    for local in &mut post.locals {
        if !local.is_this {
            local.parameter = None;
        }
    }
    let post_state_local = post.locals.next_id();
    post.locals.push(Local {
        name_hint: None,
        name: "$state".into(),
        ty: carrier_type.clone(),
        parameter: Some(0),
        is_this: false,
        needs_default: false,
    });
    let post_state = add_value(
        &mut post,
        ValueKind::Parameter {
            slot: 1,
            index: Some(0),
        },
        carrier_type,
        call.block,
        Some(post_state_local),
    );
    let mut restore = Vec::new();
    for &local in &captures {
        let ty = post.locals[local].ty.clone();
        let value = add_value(
            &mut post,
            ValueKind::Field {
                field: carrier_field(&carrier, format!("v{local}"), &ty),
                receiver: Some(post_state),
            },
            ty,
            call.block,
            None,
        );
        restore.push(Statement::Assign {
            local,
            value: crate::ir::ValueUse::Reference(value),
        });
    }
    restore.append(&mut post.blocks[call.block].statements);
    post.blocks[call.block].statements = restore;

    let mut declarations = CodeWriter::new(1);
    let throws = throws_clause(program.method_throws(member), types);
    let parameters = parameter_declarations(ir, &signature, types);
    declarations.open(format!(
        "private static {carrier} {prepare}({parameters}){throws} {{"
    ));
    let pre_body = cx.render(
        &PreparedIr::new(pre),
        Some(member),
        BodyOptions::original(BodyKind::Helper),
    )?;
    declarations.text.push_str(&pre_body.text);
    declarations.close("}");
    declarations.line("");

    let bridge_arity = bridge_arity(program, types)?;
    let mut markers = String::new();
    for index in 1..bridge_arity {
        write!(markers, ", boolean $marker{index}").unwrap();
    }
    let simple = types
        .names
        .class_name(types.owner)
        .rsplit('/')
        .next()
        .unwrap();
    declarations.open(format!(
        "private {simple}({carrier} $state{markers}){throws} {{"
    ));
    let keyword = if call.target.owner == program.class(types.owner).name {
        "this"
    } else {
        "super"
    };
    declarations.line(format!(
        "{keyword}({});",
        (0..call.args.len())
            .map(|index| format!("$state.a{index}"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    if keyword == "super" {
        for assignment in default_fields {
            declarations.line(assignment);
        }
    }
    // The carrier replaced the original descriptor, so its parameter indexes
    // cannot consume the original constructor's incoming Boolean facts.
    let post_options = BodyOptions::captured(BodyKind::Constructor {
        delegation: None,
        delegates: keyword == "this",
    });
    let post_body = cx.render(&PreparedIr::new(post), Some(member), post_options)?;
    declarations.text.push_str(&post_body.text);
    declarations.close("}");
    declarations.line("");
    declarations.open(format!("private static final class {carrier} {{"));
    for (index, ty) in target_signature.parameters.iter().enumerate() {
        declarations.line(format!("{} a{index};", types.ty(ty)));
    }
    for &local in &captures {
        declarations.line(format!("{} v{local};", types.ty(&ir.locals[local].ty)));
    }
    declarations.close("}");

    let arguments = parameter_names(ir, &signature).join(", ");
    let marker_values = ", false".repeat(bridge_arity - 1);
    let mut body = CodeWriter::new(2);
    body.line(format!("this({prepare}({arguments}){marker_values});"));
    Ok(super::RenderedMethod {
        body: body.text,
        declarations: declarations.text,
        fallback: pre_body.fallback || post_body.fallback,
    })
}

fn direct_arguments(ir: &MethodIr, call: &Delegation) -> Option<MethodIr> {
    let mut inline = ir.clone();
    let mut block = ir.entry;
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(block) || !ir.blocks[block].handlers.is_empty() {
            return None;
        }
        let end = if block == call.block {
            call.statement
        } else {
            ir.blocks[block].statements.len()
        };
        for statement in &ir.blocks[block].statements[..end] {
            let Statement::Assign { local, value } = statement else {
                return None;
            };
            if !pure_value(&inline, value.value(), true, &mut HashSet::new()) {
                return None;
            }
            // Copies into a phi local have a different RHS identity; definitions
            // already own theirs. Both can be rendered through the pure RHS.
            for id in inline.values.ids() {
                if inline.values[id].local == Some(*local) {
                    inline.values[id].local = None;
                    if id != value.value() {
                        inline.values[id].kind = ValueKind::Alias(value.value());
                    }
                }
            }
        }
        if block == call.block {
            // The direct candidate must not erase the scheduler's storage
            // protection into a deeply recursive expression-binding request.
            let mut pending: Vec<_> = call
                .args
                .iter()
                .map(|&value| (crate::ir::scheduled::ValueUse::Reference(value), 0))
                .collect();
            while let Some((value, depth)) = pending.pop() {
                if depth >= MAX_DIRECT_ARGUMENT_DEPTH {
                    return None;
                }
                value.for_each_child(&inline, |child| pending.push((child, depth + 1)));
            }
            return Some(inline);
        }
        let Terminator::Goto(next) = ir.blocks[block].terminator else {
            return None;
        };
        block = next;
    }
}

const MAX_DIRECT_ARGUMENT_DEPTH: usize = 48;

fn pure_value(ir: &MethodIr, value: ValueId, root: bool, seen: &mut HashSet<ValueId>) -> bool {
    if seen.len() >= MAX_DIRECT_ARGUMENT_DEPTH || !seen.insert(value) {
        return false;
    }
    if !root && let Some(local) = ir.values[value].local {
        let pure = ir.locals[local].parameter.is_some() || ir.locals[local].is_this;
        seen.remove(&value);
        return pure;
    }
    let pure = match &ir.values[value].kind {
        ValueKind::Parameter { .. } | ValueKind::Literal(_) => true,
        ValueKind::Alias(_) | ValueKind::Unary { .. } | ValueKind::Compare { .. } => true,
        ValueKind::Binary { op, .. } => {
            !matches!(op, crate::ir::BinaryOp::Div | crate::ir::BinaryOp::Rem)
        }
        ValueKind::Cast { ty, .. } => !ty.is_reference(),
        _ => false,
    };
    let result = pure
        && ir.values[value]
            .kind
            .operands()
            .iter()
            .all(|&operand| pure_value(ir, operand, false, seen));
    seen.remove(&value);
    result
}

pub(super) fn is_this(ir: &MethodIr, value: ValueId, seen: &mut HashSet<ValueId>) -> bool {
    if !seen.insert(value) {
        return true;
    }
    match &ir.values[value].kind {
        ValueKind::Parameter { index: None, .. } => true,
        ValueKind::Alias(value) => is_this(ir, *value, seen),
        ValueKind::Phi { inputs, .. } => {
            !inputs.is_empty() && inputs.iter().all(|input| is_this(ir, input.value, seen))
        }
        _ => false,
    }
}

fn reachable(ir: &MethodIr, entry: BlockId, stop: Option<BlockId>) -> BTreeSet<BlockId> {
    let mut seen = BTreeSet::new();
    let mut pending = vec![entry];
    while let Some(block) = pending.pop() {
        if !seen.insert(block) || stop == Some(block) {
            continue;
        }
        pending.extend(ir.blocks[block].terminator.successors());
        pending.extend(
            ir.blocks[block]
                .handlers
                .iter()
                .map(|handler| handler.target),
        );
    }
    seen
}

fn prune(ir: &mut MethodIr) -> BTreeSet<BlockId> {
    let seen = reachable(ir, ir.entry, None);
    for (id, block) in ir.blocks.iter_enumerated_mut() {
        if !seen.contains(&id) {
            block.statements.clear();
            block.terminator = Terminator::Unreachable;
            block.handlers.clear();
        }
    }
    seen
}

fn add_value(
    ir: &mut MethodIr,
    kind: ValueKind,
    ty: JvmType,
    block: BlockId,
    local: Option<LocalId>,
) -> ValueId {
    let id = ir.values.next_id();
    ir.values.push(Value {
        local,
        kind,
        ty,
        pc: ir.blocks[block].pc,
        block,
    });
    id
}

fn local_value(ir: &MethodIr, local: LocalId) -> Option<ValueId> {
    ir.values
        .iter_enumerated()
        .find_map(|(id, value)| (value.local == Some(local)).then_some(id))
}
fn carrier_field(owner: &str, name: String, ty: &JvmType) -> Member {
    Member {
        owner: owner.into(),
        name,
        descriptor: ty.descriptor(),
    }
}

fn helper_name(program: &Program<'_>, types: &TypeNames<'_>, index: usize, prefix: &str) -> String {
    super::generated::symbol(program, types, &format!("{prefix}{index}"))
}

fn bridge_arity(program: &Program<'_>, types: &TypeNames<'_>) -> Result<usize> {
    let arities: BTreeSet<_> = program
        .class(types.owner)
        .methods
        .iter()
        .filter(|method| method.name == "<init>")
        .map(|method| {
            MethodDescriptor::parse(&method.descriptor)
                .map(|descriptor| descriptor.parameters.len())
        })
        .collect::<Result<_>>()?;
    (1..255)
        .find(|arity| !arities.contains(arity))
        .context("no unused constructor arity for a private preinitialization bridge")
}

fn throws_clause(exceptions: &[String], types: &TypeNames<'_>) -> String {
    if exceptions.is_empty() {
        String::new()
    } else {
        format!(
            " throws {}",
            exceptions
                .iter()
                .map(|name| types.class(name))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn parameter_names(ir: &MethodIr, signature: &MethodDescriptor) -> Vec<String> {
    let mut slot = 1;
    signature
        .parameters
        .iter()
        .enumerate()
        .map(|(index, ty)| {
            let name = ir
                .locals
                .iter()
                .find(|local| local.parameter == Some(index))
                .map_or_else(|| format!("p{slot}"), |local| local.name.clone());
            slot += ty.slots();
            name
        })
        .collect()
}

fn parameter_declarations(
    ir: &MethodIr,
    signature: &MethodDescriptor,
    types: &TypeNames<'_>,
) -> String {
    signature
        .parameters
        .iter()
        .zip(parameter_names(ir, signature))
        .map(|(ty, name)| format!("{} {name}", types.ty(ty)))
        .collect::<Vec<_>>()
        .join(", ")
}
