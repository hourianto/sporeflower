//! Checked exceptions visible to Java's source checker. Bytecode handlers cover
//! ranges, so a per-block dispatcher must omit an impossible checked catch on
//! a block that only performs, for example, integer arithmetic.
use crate::ir::scheduled::{self, ValueUse};
use crate::ir::{BlockId, MethodIr, Statement, ValueId, ValueKind};
use crate::program::{MemberId, Program};
use crate::structure::Stmt;
use std::collections::BTreeSet;

#[derive(Clone, Default)]
pub struct Thrown {
    pub(super) types: BTreeSet<String>,
    pub(super) unknown: bool,
    pub(super) source_types: BTreeSet<String>,
    pub(super) source_unknown: bool,
}

impl Thrown {
    pub fn caught_by(&self, program: &Program<'_>, catch: Option<&str>) -> bool {
        Self::covered(program, catch, &self.types, self.unknown)
    }

    pub fn source_caught_by(&self, program: &Program<'_>, catch: Option<&str>) -> bool {
        Self::covered(program, catch, &self.source_types, self.source_unknown)
    }

    fn covered(
        program: &Program<'_>,
        catch: Option<&str>,
        types: &BTreeSet<String>,
        unknown: bool,
    ) -> bool {
        let Some(catch) = catch else {
            return true;
        };
        unknown
            || program.id(catch).is_none()
            || program.is_subtype("java/lang/RuntimeException", catch)
            || program.is_subtype("java/lang/Error", catch)
            || program.is_subtype(catch, "java/lang/RuntimeException")
            || program.is_subtype(catch, "java/lang/Error")
            || types.iter().any(|thrown| {
                program.is_subtype(thrown, catch) || program.is_subtype(catch, thrown)
            })
    }

    pub(super) fn remove_caught(&mut self, program: &Program<'_>, catch: Option<&str>) {
        if crate::cfg::dispatch::is_catch_all(catch) {
            self.types.clear();
            self.unknown = false;
            self.source_types.clear();
            self.source_unknown = false;
        } else if let Some(catch) = catch {
            self.types.retain(|ty| !program.is_subtype(ty, catch));
            self.source_types
                .retain(|ty| !program.is_subtype(ty, catch));
        }
    }

    pub(super) fn extend(&mut self, other: Self) {
        self.types.extend(other.types);
        self.unknown |= other.unknown;
        self.source_types.extend(other.source_types);
        self.source_unknown |= other.source_unknown;
    }
}

/// Recover handler ownership using bytecode effects. Java checker witnesses and
/// escaping source effects belong to the later finalized source-body plan.
pub fn structure(
    cx: &super::context::EmissionContext<'_, '_>,
    member: Option<MemberId>,
    prepared: &crate::structure::PreparedIr,
) -> crate::Result<crate::structure::StructuredMethod> {
    let program = cx.program;
    let ir = prepared.ir();
    let filter = |block, region: Option<&[Stmt]>, handlers: &[crate::cfg::HandlerEdge]| {
        let thrown = if let Some(region) = region {
            effects(cx, member, ir, region)
        } else {
            let mut thrown = effects(cx, member, ir, &[Stmt::Basic(block)]);
            add_terminator(cx, member, ir, block, &mut thrown);
            thrown
        };
        select_handlers(program, &thrown, handlers)
    };
    crate::structure::structure_with_filter(prepared, Some(&filter))
}

fn select_handlers(
    program: &Program<'_>,
    thrown: &Thrown,
    handlers: &[crate::cfg::HandlerEdge],
) -> Vec<crate::cfg::HandlerEdge> {
    crate::cfg::dispatch::effective_with(handlers, |later, earlier| {
        program.is_subtype(later, earlier)
    })
    .filter(|handler| thrown.caught_by(program, handler.catch_type.as_deref()))
    .cloned()
    .collect()
}

/// Read-only effect queries used during ownership recovery. Child effects are
/// combined by the same rules as the final source pass below.
pub fn effects(
    cx: &super::context::EmissionContext<'_, '_>,
    member: Option<MemberId>,
    ir: &MethodIr,
    body: &[Stmt],
) -> Thrown {
    let program = cx.program;
    let mut result = Thrown::default();
    for statement in body {
        result.extend(direct_effects(cx, member, ir, statement));
        match statement {
            Stmt::Try { body, catches } => {
                let mut protected = effects(cx, member, ir, body);
                for catch in catches {
                    protected.remove_caught(program, catch.catch_type.as_deref());
                    result.extend(effects(cx, member, ir, &catch.body));
                }
                result.extend(protected);
            }
            Stmt::Dispatch { cases, .. } => {
                for case in cases {
                    let mut protected = effects(cx, member, ir, &case.body);
                    dispatch_terminator(cx, member, ir, case, &mut protected);
                    for handler in &case.handlers {
                        protected.remove_caught(program, handler.catch_type.as_deref());
                    }
                    result.extend(protected);
                }
            }
            _ => {
                for body in statement.bodies() {
                    result.extend(effects(cx, member, ir, body));
                }
            }
        }
    }
    result
}

fn dispatch_terminator(
    cx: &super::context::EmissionContext<'_, '_>,
    member: Option<MemberId>,
    ir: &MethodIr,
    case: &crate::structure::DispatchCase,
    thrown: &mut Thrown,
) {
    if !matches!(
        case.end,
        crate::structure::FlowEnd::Unreachable | crate::structure::FlowEnd::Multi(_)
    ) {
        add_terminator(cx, member, ir, case.head, thrown);
    }
}

/// Operations evaluated directly by one source statement, excluding children.
fn direct_effects(
    cx: &super::context::EmissionContext<'_, '_>,
    member: Option<MemberId>,
    ir: &MethodIr,
    statement: &Stmt,
) -> Thrown {
    let mut result = Thrown::default();
    match statement {
        Stmt::Basic(_) | Stmt::Statements { .. } => {
            for (_, statement) in statement.scheduled(ir).unwrap().iter() {
                result.extend(statement_effects(cx, member, ir, statement));
            }
        }
        Stmt::Return(block) | Stmt::Throw(block) => {
            add_terminator(cx, member, ir, *block, &mut result)
        }
        Stmt::If { condition, .. } => {
            for head in condition.heads() {
                for value in ir.blocks[head].terminator.operands() {
                    value_effects(cx, member, ir, value, &mut result);
                }
            }
        }
        Stmt::Loop {
            continuation: Some(block),
            ..
        } => result.extend(direct_effects(cx, member, ir, &Stmt::Basic(*block))),
        Stmt::Switch { head, .. } => {
            for value in ir.blocks[*head].terminator.operands() {
                value_effects(cx, member, ir, value, &mut result);
            }
        }
        Stmt::Synchronized { monitor, .. } => value_effects(cx, member, ir, *monitor, &mut result),
        _ => (),
    }
    result
}

pub fn add_terminator(
    cx: &super::context::EmissionContext<'_, '_>,
    member: Option<MemberId>,
    ir: &MethodIr,
    block: BlockId,
    result: &mut Thrown,
) {
    let program = cx.program;
    for value in ir.blocks[block].terminator.operands() {
        value_effects(cx, member, ir, value, result);
    }
    if let crate::ir::Terminator::Throw { value, pc } = ir.blocks[block].terminator {
        throw_effects(program, member, ir, value, pc, result);
    }
}

pub(super) fn throw_effects(
    program: &Program<'_>,
    member: Option<MemberId>,
    ir: &MethodIr,
    value: ValueId,
    pc: u32,
    result: &mut Thrown,
) {
    if let Some(fact) =
        member.and_then(|member| program.throws_plan().throw_at_instruction(member, pc))
    {
        result.types.extend(fact.checked.iter().cloned());
        result.unknown |= fact.unknown;
        result.source_types.extend(fact.checked.iter().cloned());
        return;
    }
    if let crate::classfile::JvmType::Object(name) = super::conversions::source_type(ir, value) {
        result.types.insert(name.clone());
        result.source_types.insert(name.clone());
    }
}

pub(super) fn statement_effects(
    cx: &super::context::EmissionContext<'_, '_>,
    member: Option<MemberId>,
    ir: &MethodIr,
    statement: &Statement,
) -> Thrown {
    let mut result = Thrown::default();
    if let Statement::ConstructorCall { method, .. } = statement {
        call_effects(
            cx,
            &method.owner,
            &method.name,
            &method.descriptor,
            crate::ir::InvokeKind::Special,
            &mut result,
        );
    }
    operation_effects(
        cx,
        member,
        ir,
        scheduled::statement_values(ir, statement).operations(),
        &mut result,
    );
    result
}

pub(super) fn expression_effects(
    cx: &super::context::EmissionContext<'_, '_>,
    member: Option<MemberId>,
    ir: &MethodIr,
    usage: ValueUse,
    result: &mut Thrown,
) {
    operation_effects(
        cx,
        member,
        ir,
        scheduled::values(ir, usage).operations(),
        result,
    );
}

fn value_effects(
    cx: &super::context::EmissionContext<'_, '_>,
    member: Option<MemberId>,
    ir: &MethodIr,
    value: ValueId,
    result: &mut Thrown,
) {
    operation_effects(
        cx,
        member,
        ir,
        scheduled::values(ir, ValueUse::Reference(value)).operations(),
        result,
    );
}

fn operation_effects(
    cx: &super::context::EmissionContext<'_, '_>,
    member: Option<MemberId>,
    ir: &MethodIr,
    values: impl Iterator<Item = ValueId>,
    result: &mut Thrown,
) {
    let program = cx.program;
    for value in values {
        match &ir.values[value].kind {
            ValueKind::Call { kind, method, .. } => {
                let reflection = (method.owner == "java/lang/Class"
                    && method.name == "newInstance"
                    && method.descriptor == "()Ljava/lang/Object;")
                    .then(|| {
                        member.and_then(|member| {
                            program
                                .throws_plan()
                                .reflection_at(member, ir.values[value].pc)
                        })
                    })
                    .flatten();
                if let Some(reflection) = reflection {
                    result.types.extend(reflection.checked.iter().cloned());
                    result.unknown |= reflection.unknown;
                    if let Some(target) =
                        program.resolve_method(&method.owner, &method.name, &method.descriptor)
                    {
                        result
                            .source_types
                            .extend(program.method_throws(target).iter().cloned());
                    } else {
                        result.source_unknown = true;
                    }
                } else {
                    call_effects(
                        cx,
                        &method.owner,
                        &method.name,
                        &method.descriptor,
                        *kind,
                        result,
                    );
                }
            }
            ValueKind::New {
                class, descriptor, ..
            } => call_effects(
                cx,
                class,
                "<init>",
                descriptor,
                crate::ir::InvokeKind::Special,
                result,
            ),
            _ => {}
        }
    }
}

fn call_effects(
    cx: &super::context::EmissionContext<'_, '_>,
    owner: &str,
    name: &str,
    descriptor: &str,
    kind: crate::ir::InvokeKind,
    result: &mut Thrown,
) {
    let program = cx.program;
    if let Some(contract) = cx.generated.borrow().get(owner, name, descriptor) {
        result.extend(contract.effects.clone());
        return;
    }
    let target = if kind == crate::ir::InvokeKind::Special {
        program.exact_method(
            Some(cx.types.owner),
            kind,
            &crate::ir::Member {
                owner: owner.into(),
                name: name.into(),
                descriptor: descriptor.into(),
            },
        )
    } else {
        program.resolve_method(owner, name, descriptor)
    };
    if let Some(member) = target {
        let effects = program.throws_plan().call_effects(
            member,
            owner,
            matches!(
                kind,
                crate::ir::InvokeKind::Virtual | crate::ir::InvokeKind::Interface
            ),
        );
        result.types.extend(effects.checked.iter().cloned());
        result.unknown |= effects.unknown;
        result
            .source_types
            .extend(source_call_throws(program, member, owner, name, descriptor));
    } else {
        result.unknown = true;
        result.source_unknown = true;
    }
}

/// Java checks all maximally specific inherited declarations at an interface
/// call. JVM resolution may choose one of them, but its throws clause alone is
/// not the source contract (for example, Wide.run + Runnable.run throws none).
fn source_call_throws(
    program: &Program<'_>,
    resolved: MemberId,
    owner: &str,
    name: &str,
    descriptor: &str,
) -> Vec<String> {
    let declarations = program.method_throws(resolved);
    let Some(owner) = program.id(owner) else {
        return declarations.to_vec();
    };
    if resolved.class == owner || program.class(resolved.class).access_flags & 0x0200 == 0 {
        return declarations.to_vec();
    }
    let inherited: Vec<_> = std::iter::once(owner)
        .chain(program.ancestors(owner).iter().copied())
        .filter(|&class| program.class(class).access_flags & 0x0200 != 0)
        .filter_map(|class| program.declared_method(class, name, descriptor))
        .collect();
    let specific: Vec<_> = inherited
        .iter()
        .copied()
        .filter(|candidate| {
            !inherited.iter().any(|other| {
                other.class != candidate.class
                    && program.is_subtype(
                        &program.class(other.class).name,
                        &program.class(candidate.class).name,
                    )
            })
        })
        .collect();
    if specific.len() <= 1 {
        return declarations.to_vec();
    }
    specific
        .iter()
        .flat_map(|&member| program.method_throws(member))
        .filter(|exception| {
            specific.iter().all(|&member| {
                program
                    .method_throws(member)
                    .iter()
                    .any(|bound| program.is_subtype(exception, bound))
            })
        })
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
