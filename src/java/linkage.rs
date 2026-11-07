//! Source equivalents for exact member references which cannot resolve.
//!
//! Resolution errors occur after evaluating the operands, before dereferencing
//! a receiver or initializing an invokestatic target. Keep the throw in a typed
//! caller-local helper; adding a method to the target would change those rules.
use super::{body::CodeWriter, literals, types::TypeNames};
use crate::Result;
use crate::classfile::{Constant, JvmType, MethodDescriptor};
use crate::ir::{
    BinaryOp, InvokeKind, Literal, Member, MethodIr, Statement, UpdateTarget, Value, ValueKind,
};
use crate::program::{ClassId, MemberId, MemberResolution, Program};
use anyhow::ensure;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Target {
    owner: String,
    name: String,
    descriptor: String,
    receiver: bool,
    operation: Operation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Operation {
    Method,
    Read,
    Write,
    /// A failing write after a successful read and update computation.
    WriteValue,
}

struct Helper {
    target: Target,
    name: String,
    descriptor: MethodDescriptor,
    instantiable: bool,
    error: &'static str,
    exact_error: &'static str,
    final_field: Option<MemberId>,
}

pub(super) struct LinkagePlan {
    owner: String,
    by_target: BTreeMap<Target, usize>,
    helpers: Vec<Helper>,
}

impl LinkagePlan {
    pub fn new(program: &Program<'_>, types: &TypeNames<'_>) -> Self {
        let class = program.class(types.owner);
        let mut targets = BTreeMap::new();
        // Resolve each constant-pool identity once. Reserve both invocation
        // shapes without decoding every method again; only used helpers emit.
        for (index, constant) in class.constant_pool.entries.iter().enumerate() {
            if !matches!(
                constant,
                Constant::Fieldref { .. }
                    | Constant::Methodref { .. }
                    | Constant::InterfaceMethodref { .. }
            ) {
                continue;
            }
            // Obfuscators may retain invalid unused pool entries. The lifter
            // diagnoses an invalid reference if executable bytecode uses it.
            let Ok(member) = class.constant_pool.member_ref(index as u16) else {
                continue;
            };
            let Some(target) = program.id(member.owner) else {
                continue;
            };
            let field = matches!(constant, Constant::Fieldref { .. });
            if !program.is_own(target) && !field {
                continue;
            }
            let resolution = if field {
                program.field_resolution(member.owner, member.name, member.descriptor)
            } else if member.name == "<init>" {
                program
                    .declared_method(target, member.name, member.descriptor)
                    .map_or(MemberResolution::Missing, MemberResolution::Found)
            } else {
                program.method_resolution(member.owner, member.name, member.descriptor)
            };
            let error = match resolution {
                MemberResolution::Incomplete => None,
                MemberResolution::Missing if !program.is_own(target) => None,
                MemberResolution::Missing => Some(if field {
                    "java/lang/NoSuchFieldError"
                } else {
                    "java/lang/NoSuchMethodError"
                }),
                MemberResolution::Found(found) => {
                    let access = if field {
                        program.field(found).access_flags
                    } else {
                        program.method(found).access_flags
                    };
                    (!program.member_accessible_from(access, found.class, types.owner))
                        .then_some("java/lang/IllegalAccessError")
                }
            };
            for receiver in [false, true]
                .into_iter()
                .filter(|&receiver| !receiver || member.name != "<init>")
            {
                for &operation in if field {
                    &[Operation::Read, Operation::Write, Operation::WriteValue][..]
                } else {
                    &[Operation::Method][..]
                } {
                    let final_field = if field
                        && matches!(operation, Operation::Write | Operation::WriteValue)
                    {
                        match resolution {
                            MemberResolution::Found(found)
                                if program.field(found).access_flags & 0x0010 != 0
                                    && (found.class != types.owner
                                        || class.major_version >= 53)
                                    && receiver
                                        == (program.field(found).access_flags & 0x0008 == 0) =>
                            {
                                Some(found)
                            }
                            _ => None,
                        }
                    } else {
                        None
                    };
                    let Some((error, final_field)) =
                        error.map(|error| (error, None)).or_else(|| {
                            final_field.map(|field| ("java/lang/IllegalAccessError", Some(field)))
                        })
                    else {
                        continue;
                    };
                    targets.insert(
                        Target {
                            owner: member.owner.into(),
                            name: member.name.into(),
                            descriptor: member.descriptor.into(),
                            receiver,
                            operation,
                        },
                        (error, final_field),
                    );
                }
            }
        }
        let mut helpers = Vec::new();
        let mut by_target = BTreeMap::new();
        for (target, (exact_error, final_field)) in targets {
            let descriptor = match target.operation {
                Operation::Method => target.descriptor.clone(),
                Operation::Read => format!("(){}", target.descriptor),
                Operation::Write => format!("({})V", target.descriptor),
                Operation::WriteValue => format!("({0}){0}", target.descriptor),
            };
            let Ok(mut descriptor) = MethodDescriptor::parse(&descriptor) else {
                continue;
            };
            let index = helpers.len();
            let name = super::generated::symbol(program, types, &format!("$sf$linkage_{index}"));
            if target.receiver && target.operation != Operation::WriteValue {
                descriptor
                    .parameters
                    .insert(0, JvmType::Object(target.owner.clone()));
            }
            if target.name == "<init>" {
                descriptor.return_type = JvmType::Object(target.owner.clone());
            }
            by_target.insert(target.clone(), index);
            let instantiable = target.name != "<init>"
                || program
                    .class(program.id(&target.owner).unwrap())
                    .access_flags
                    & 0x0600
                    == 0;
            helpers.push(Helper {
                target,
                name,
                descriptor,
                instantiable,
                error: supported_error(program, exact_error),
                exact_error,
                final_field,
            });
        }
        Self {
            owner: class.name.clone(),
            by_target,
            helpers,
        }
    }

    pub fn lower(
        &self,
        cx: &super::context::EmissionContext<'_, '_>,
        member: MemberId,
        ir: &mut MethodIr,
    ) -> Result<()> {
        if self.helpers.is_empty() {
            return Ok(());
        }
        for block in &ir.blocks {
            for statement in &block.statements {
                if let Statement::ConstructorCall { method, .. } = statement {
                    ensure!(
                        !self.by_target.contains_key(&Target {
                            owner: method.owner.clone(),
                            name: method.name.clone(),
                            descriptor: method.descriptor.clone(),
                            receiver: false,
                            operation: Operation::Method,
                        }),
                        "unresolved constructor delegation {}.{}{} cannot be represented as a Java first statement",
                        method.owner,
                        method.name,
                        method.descriptor
                    );
                }
            }
        }
        // Updating a fused expression appends its field read and arithmetic.
        // Visit only the original values; those appended reads are already valid.
        for id in (0..ir.values.len()).map(crate::ir::ValueId::from_index) {
            let (mut target, mut args) = match &ir.values[id].kind {
                ValueKind::Call {
                    method,
                    receiver,
                    args,
                    ..
                } => (
                    Target {
                        owner: method.owner.clone(),
                        name: method.name.clone(),
                        descriptor: method.descriptor.clone(),
                        receiver: receiver.is_some(),
                        operation: Operation::Method,
                    },
                    receiver
                        .iter()
                        .copied()
                        .chain(args.iter().copied())
                        .collect(),
                ),
                ValueKind::New {
                    class,
                    descriptor,
                    args,
                    ..
                } => (
                    Target {
                        owner: class.clone(),
                        name: "<init>".into(),
                        descriptor: descriptor.clone(),
                        receiver: false,
                        operation: Operation::Method,
                    },
                    args.clone(),
                ),
                ValueKind::Field { field, receiver }
                | ValueKind::Update {
                    target: UpdateTarget::Field { field, receiver },
                    ..
                }
                | ValueKind::CompoundUpdate {
                    target: UpdateTarget::Field { field, receiver },
                    ..
                } => (
                    Target {
                        owner: field.owner.clone(),
                        name: field.name.clone(),
                        descriptor: field.descriptor.clone(),
                        receiver: receiver.is_some(),
                        operation: Operation::Read,
                    },
                    receiver.iter().copied().collect(),
                ),
                _ => continue,
            };
            let index = if let Some(index) = self.helper(cx.program, member, &target) {
                index
            } else if target.operation == Operation::Read
                && matches!(
                    ir.values[id].kind,
                    ValueKind::Update { .. } | ValueKind::CompoundUpdate { .. }
                )
            {
                target.operation = Operation::WriteValue;
                let Some(index) = self.helper(cx.program, member, &target) else {
                    continue;
                };
                args = vec![update_computation(ir, id)];
                index
            } else {
                continue;
            };
            let helper = &self.helpers[index];
            ensure!(
                helper.instantiable,
                "unresolved constructor on abstract/interface target {} has an earlier NEW instantiation failure",
                helper.target.owner
            );
            ir.values[id].kind = ValueKind::Call {
                kind: InvokeKind::Static,
                method: Member {
                    owner: self.owner.clone(),
                    name: helper.name.clone(),
                    descriptor: helper.descriptor.descriptor(),
                },
                receiver: None,
                args,
            };
            let ValueKind::Call { method, .. } = &ir.values[id].kind else {
                unreachable!()
            };
            // Preserve lexical handler eligibility while exposing the exact Java
            // declaration (no checked throws) to source catch binding.
            cx.register(
                method,
                super::exceptions::Thrown {
                    types: [helper.error.to_owned()].into(),
                    unknown: true,
                    ..Default::default()
                },
            )?;
        }
        for (block_id, block) in ir.blocks.iter_enumerated_mut() {
            for statement in &mut block.statements {
                let Statement::PutField {
                    pc,
                    field,
                    receiver,
                    value,
                } = statement
                else {
                    continue;
                };
                let target = Target {
                    owner: field.owner.clone(),
                    name: field.name.clone(),
                    descriptor: field.descriptor.clone(),
                    receiver: receiver.is_some(),
                    operation: Operation::Write,
                };
                let Some(index) = self.helper(cx.program, member, &target) else {
                    continue;
                };
                let helper = &self.helpers[index];
                let method = Member {
                    owner: self.owner.clone(),
                    name: helper.name.clone(),
                    descriptor: helper.descriptor.descriptor(),
                };
                cx.register(
                    &method,
                    super::exceptions::Thrown {
                        types: [helper.error.to_owned()].into(),
                        unknown: true,
                        ..Default::default()
                    },
                )?;
                let evaluation = ir.values.push(Value {
                    local: None,
                    ty: JvmType::Void,
                    pc: pc.unwrap_or(ir.values[*value].pc),
                    block: block_id,
                    kind: ValueKind::Call {
                        kind: InvokeKind::Static,
                        method,
                        receiver: None,
                        args: receiver
                            .iter()
                            .copied()
                            .chain(std::iter::once(*value))
                            .collect(),
                    },
                });
                *statement = Statement::Eval(evaluation);
            }
        }
        Ok(())
    }

    fn helper(&self, program: &Program<'_>, member: MemberId, target: &Target) -> Option<usize> {
        let &index = self.by_target.get(target)?;
        (!self.helpers[index]
            .final_field
            .is_some_and(|field| program.field_write_permitted(field, member)))
        .then_some(index)
    }

    pub fn diagnostic(&self, cx: &super::context::EmissionContext<'_, '_>) -> Option<String> {
        let missing: BTreeSet<_> = self
            .helpers
            .iter()
            .filter(|helper| self.used(cx, helper) && helper.error != helper.exact_error)
            .map(|helper| {
                format!(
                    "{} uses {}",
                    helper.exact_error.replace('/', "."),
                    helper.error.replace('/', ".")
                )
            })
            .collect();
        (!missing.is_empty()).then(|| format!(
            "{}: exact member linkage failures use the nearest supported error because the selected API omits the specific errors (CLDC 1.1 section 4.2): {}",
            self.owner, missing.into_iter().collect::<Vec<_>>().join("; ")
        ))
    }

    fn used(&self, cx: &super::context::EmissionContext<'_, '_>, helper: &Helper) -> bool {
        cx.generated
            .borrow()
            .get(&self.owner, &helper.name, &helper.descriptor.descriptor())
            .is_some()
    }

    pub fn emit(&self, cx: &super::context::EmissionContext<'_, '_>, out: &mut CodeWriter) {
        let types = cx.types;
        for helper in self.helpers.iter().filter(|helper| self.used(cx, helper)) {
            let parameters = helper
                .descriptor
                .parameters
                .iter()
                .enumerate()
                .map(|(index, ty)| format!("{} p{index}", types.ty(ty)))
                .collect::<Vec<_>>()
                .join(", ");
            out.line("");
            out.open(format!(
                "private static {} {}({parameters}) {{",
                types.ty(&helper.descriptor.return_type),
                helper.name
            ));
            let message = format!(
                "{}.{}{}",
                helper.target.owner.replace('/', "."),
                helper.target.name,
                helper.target.descriptor
            );
            out.line(format!(
                "throw new {}({});",
                helper.error.replace('/', "."),
                literals::string_literal(&message.encode_utf16().collect::<Vec<_>>())
            ));
            out.close("}");
        }
    }
}

/// Keep a valid field read before the RHS and arithmetic of a fused update.
/// The write helper accepts only the computed value, so the receiver is evaluated
/// exactly once by that read; no assignment to the source final is emitted.
fn update_computation(ir: &mut MethodIr, id: crate::ir::ValueId) -> crate::ir::ValueId {
    let value = ir.values[id].clone();
    let (target, op, right, operation_type) = match value.kind {
        ValueKind::CompoundUpdate {
            target,
            op,
            right,
            operation_type,
        } => (target, op, right, operation_type),
        ValueKind::Update {
            target, increment, ..
        } => {
            let right = ir.values.push(Value {
                local: None,
                kind: ValueKind::Literal(Literal::Int(1)),
                ty: JvmType::Int,
                pc: value.pc,
                block: value.block,
            });
            let operation_type = match value.ty {
                JvmType::Boolean | JvmType::Byte | JvmType::Char | JvmType::Short => JvmType::Int,
                _ => value.ty.clone(),
            };
            (
                target,
                if increment {
                    BinaryOp::Add
                } else {
                    BinaryOp::Sub
                },
                right,
                operation_type,
            )
        }
        _ => unreachable!(),
    };
    let UpdateTarget::Field { field, receiver } = target else {
        unreachable!()
    };
    let read = ir.values.push(Value {
        local: None,
        kind: ValueKind::Field { field, receiver },
        ty: value.ty,
        pc: value.pc,
        block: value.block,
    });
    ir.values.push(Value {
        local: None,
        kind: ValueKind::Binary {
            op,
            left: read,
            right,
        },
        ty: operation_type,
        pc: value.pc,
        block: value.block,
    })
}

fn supported_error(program: &Program<'_>, exact: &'static str) -> &'static str {
    [
        exact,
        "java/lang/IncompatibleClassChangeError",
        "java/lang/LinkageError",
        "java/lang/Error",
    ]
    .into_iter()
    .find(|name| program.id(name).is_some())
    .unwrap_or(exact)
}

/// These failures prevent loading the original class; inventing a different
/// hierarchy would make previously unreachable methods executable.
pub(super) fn validate_class(program: &Program<'_>, owner: ClassId) -> Result<()> {
    let class = program.class(owner);
    if let Some(parent) = class
        .super_name
        .as_deref()
        .and_then(|name| program.id(name))
    {
        ensure!(
            program.class(parent).access_flags & 0x10 == 0,
            "class linkage failure: {} extends final superclass {}",
            class.name,
            program.class(parent).name
        );
    }
    for method in &class.methods {
        if method.name.starts_with('<') || method.access_flags & 0x000a != 0 {
            continue;
        }
        for &ancestor in program.ancestors(owner) {
            if let Some(inherited) =
                program.declared_method(ancestor, &method.name, &method.descriptor)
            {
                let inherited = program.method(inherited);
                ensure!(
                    inherited.access_flags & 0x001a != 0x0010
                        || !program.member_accessible_from(inherited.access_flags, ancestor, owner),
                    "class linkage failure: {}.{}{} overrides final method in {}",
                    class.name,
                    method.name,
                    method.descriptor,
                    program.class(ancestor).name
                );
            }
        }
    }
    Ok(())
}
