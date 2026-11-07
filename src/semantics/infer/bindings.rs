//! Member contracts are resolved through the same program binding rules
//! for heap publication, value production, and consumer requirements.
use crate::classfile::{JvmType, MethodDescriptor};
use crate::ir::{Member, MethodIr, ValueKind};
use crate::mappings::{Contract, MappingPlan, Target};
use crate::program::{MemberId, Program};

#[derive(Clone, Copy)]
pub(super) struct Bindings<'a, 'p> {
    pub program: &'a Program<'p>,
    pub mappings: &'a MappingPlan,
    pub member: MemberId,
}

impl Bindings<'_, '_> {
    fn method_id(&self, method: &Member) -> Option<MemberId> {
        self.program
            .resolve_method(&method.owner, &method.name, &method.descriptor)
    }
    pub(super) fn field(&self, field: &Member) -> Contract {
        self.program
            .resolve_field(&field.owner, &field.name, &field.descriptor)
            .and_then(|field| self.mappings.contract(Target::Field(field)))
            .cloned()
            .unwrap_or_default()
    }
    pub(super) fn method(&self, method: &Member, index: Option<usize>) -> Contract {
        let mut contract = self
            .method_id(method)
            .and_then(|method| {
                self.mappings
                    .contract(
                        index.map_or(Target::Return(method), |index| Target::Parameter {
                            method,
                            index,
                        }),
                    )
            })
            .cloned()
            .unwrap_or_default();
        if let Some(index) = index {
            contract.class_name |= crate::semantics::class_names::is_argument(
                self.program,
                self.mappings,
                method,
                index,
            );
        }
        contract
    }

    pub(super) fn has_evidence(&self, ir: &MethodIr) -> bool {
        let Self {
            program,
            mappings,
            member,
        } = *self;
        if mappings.contract(Target::Return(member)).is_some()
            || ir.parameters.iter().any(|&value| {
                let ValueKind::Parameter {
                    index: Some(index), ..
                } = ir.values[value].kind
                else {
                    return false;
                };
                mappings
                    .contract(Target::Parameter {
                        method: member,
                        index,
                    })
                    .is_some()
            })
            || mappings.calls().keys().any(|call| call.method == member)
        {
            return true;
        }
        let method_bound = |method: &Member| {
            is_for_name(method)
                || program
                    .class_name_consumers()
                    .has_arguments(program, method)
                || program
                    .resolve_method(&method.owner, &method.name, &method.descriptor)
                    .is_some_and(|method| {
                        mappings.contract(Target::Return(method)).is_some()
                            || MethodDescriptor::parse(&program.method(method).descriptor)
                                .is_ok_and(|descriptor| {
                                    (0..descriptor.parameters.len()).any(|index| {
                                        mappings
                                            .contract(Target::Parameter { method, index })
                                            .is_some()
                                    })
                                })
                    })
        };
        for value in &ir.values {
            match &value.kind {
                ValueKind::Field { field, .. }
                | ValueKind::Update {
                    target: crate::ir::UpdateTarget::Field { field, .. },
                    ..
                }
                | ValueKind::CompoundUpdate {
                    target: crate::ir::UpdateTarget::Field { field, .. },
                    ..
                } if program
                    .resolve_field(&field.owner, &field.name, &field.descriptor)
                    .is_some_and(|field| {
                        mappings.contract(Target::Field(field)).is_some()
                            || mappings.real_value(field).is_some()
                    }) =>
                {
                    return true;
                }
                ValueKind::Call { method, .. } if method_bound(method) => return true,
                ValueKind::New {
                    class, descriptor, ..
                } if method_bound(&Member {
                    owner: class.clone(),
                    name: "<init>".into(),
                    descriptor: descriptor.clone(),
                }) =>
                {
                    return true;
                }
                _ => {}
            }
        }
        for block in &ir.blocks {
            for statement in &block.statements {
                match statement {
                    crate::ir::Statement::PutField { field, .. }
                        if program
                            .resolve_field(&field.owner, &field.name, &field.descriptor)
                            .is_some_and(|field| {
                                mappings.contract(Target::Field(field)).is_some()
                            }) =>
                    {
                        return true;
                    }
                    crate::ir::Statement::ConstructorCall { method, .. }
                        if method_bound(method) =>
                    {
                        return true;
                    }
                    _ => {}
                }
            }
        }
        false
    }
}

use crate::semantics::class_names::is_for_name;

pub(super) fn is_wrapper(owner: &str) -> bool {
    wrapper_type(owner).is_some()
}
pub(super) fn wrapper_type(owner: &str) -> Option<JvmType> {
    Some(match owner {
        "java/lang/Byte" => JvmType::Byte,
        "java/lang/Short" => JvmType::Short,
        "java/lang/Character" => JvmType::Char,
        "java/lang/Integer" => JvmType::Int,
        "java/lang/Long" => JvmType::Long,
        _ => return None,
    })
}
pub(super) fn is_unboxing(method: &Member) -> bool {
    is_wrapper(&method.owner)
        && matches!(
            method.name.as_str(),
            "byteValue" | "shortValue" | "charValue" | "intValue" | "longValue"
        )
        && method.descriptor.starts_with("()")
}
