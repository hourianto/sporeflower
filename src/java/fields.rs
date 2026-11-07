//! JVM final fields may legally retain their VM default without any putfield or
//! putstatic. Java requires an initializer; preserve nonconstant reads by using
//! a tiny pure helper for primitive fields instead of introducing ConstantValue.
use super::{body::default_value, types::TypeNames};
use crate::{
    Result,
    classfile::{Constant, JvmType},
    ir::{Literal, Member, MethodIr, Statement, ValueId, ValueKind},
    program::{MemberId, Program},
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Default)]
pub struct FieldPlan {
    pub defaults: HashMap<usize, String>,
    instance_defaults: Vec<(usize, String)>,
    pub helpers: Vec<String>,
}

/// Constant-expression facts must be stable across independently emitted
/// classes and method order. A ConstantValue field with original stores can
/// either retain final after redundant-store removal or lose it during source
/// assignment analysis; neither result is known while binding another member.
pub(super) enum ConstantField {
    Known(u16),
    Unknown,
    Nonconstant,
}

pub(super) fn constant_field(program: &Program<'_>, id: MemberId) -> ConstantField {
    let field = program.field(id);
    if field.access_flags & 0x0010 == 0
        || program.is_own(id.class)
            && program.class(id.class).access_flags & 0x0200 == 0
            && !program.initialization_plan().permits_final(id)
    {
        return ConstantField::Nonconstant;
    }
    match field
        .constant_value
        .filter(|_| field.access_flags & 0x0008 != 0)
    {
        Some(constant)
            if !program.is_own(id.class) || !program.initialization_plan().writes_final(id) =>
        {
            ConstantField::Known(constant)
        }
        _ => ConstantField::Unknown,
    }
}

/// The JVM installs ConstantValue before running any superclass initializer.
/// A mutable Java declaration cannot express that timing. A default value needs
/// no store at all; otherwise require a quiet superclass initialization chain.
pub(super) fn constant_initializer(
    program: &Program<'_>,
    id: MemberId,
    flags: u16,
) -> Result<Option<u16>> {
    let field = program.field(id);
    let Some(constant) = field
        .constant_value
        .filter(|_| field.access_flags & 0x0008 != 0)
    else {
        return Ok(None);
    };
    if flags & 0x0010 == 0 {
        let class = program.class(id.class);
        let is_default = match class.constant_pool.get(constant)? {
            Constant::Integer(value) => *value == 0,
            Constant::Long(value) => *value == 0,
            Constant::Float(value) => value.to_bits() == 0,
            Constant::Double(value) => value.to_bits() == 0,
            _ => false,
        };
        if is_default {
            return Ok(None);
        }
        anyhow::ensure!(
            crate::initialization::superclass_initialization_is_quiet(program, id.class),
            "cannot preserve early ConstantValue of mutable field {}.{} before potentially observable superclass initialization",
            class.name,
            field.name,
        );
    }
    Ok(Some(constant))
}

/// Some old compilers repeat ConstantValue writes in <clinit>. Their RHS is a
/// literal and the field already contains that exact value before user code can
/// run. Keeping both the declaration and repeated write is illegal Java final
/// assignment; dropping this idempotent write preserves its observable effects.
pub fn redundant_constant_store(
    program: &Program<'_>,
    owner: usize,
    ir: &MethodIr,
    field: &Member,
    receiver: Option<ValueId>,
    value: ValueId,
) -> bool {
    if receiver.is_some() {
        return false;
    }
    let Some(id) = program.resolve_field(&field.owner, &field.name, &field.descriptor) else {
        return false;
    };
    if id.class != owner {
        return false;
    }
    let declaration = program.field(id);
    if declaration.access_flags & 0x0018 != 0x0018
        || !program.initialization_plan().permits_final(id)
    {
        return false;
    }
    let Some(constant) = declaration.constant_value else {
        return false;
    };
    let pool = &program.class(owner).constant_pool;
    if !same_literal(pool, constant, &ir.values[value].kind) {
        return false;
    }
    // A differing write earlier in the initializer would make resetting the
    // ConstantValue observable. All writes to this field must agree.
    ir.blocks
        .iter()
        .flat_map(|block| &block.statements)
        .all(|statement| {
            let Statement::PutField {
                field: other,
                receiver: None,
                value: other_value,
                ..
            } = statement
            else {
                return true;
            };
            program.resolve_field(&other.owner, &other.name, &other.descriptor) != Some(id)
                || same_literal(pool, constant, &ir.values[*other_value].kind)
        })
}

fn same_literal(pool: &crate::classfile::ConstantPool, constant: u16, value: &ValueKind) -> bool {
    let ValueKind::Literal(literal) = value else {
        return false;
    };
    match (pool.get(constant), literal) {
        (Ok(Constant::Integer(a)), Literal::Int(b)) => a == b,
        (Ok(Constant::Long(a)), Literal::Long(b)) => a == b,
        (Ok(Constant::Float(a)), Literal::Float(b)) => a.to_bits() == b.to_bits(),
        (Ok(Constant::Double(a)), Literal::Double(b)) => a.to_bits() == b.to_bits(),
        (Ok(Constant::String(_)), Literal::String(value)) => pool
            .string_utf16(constant)
            .is_ok_and(|expected| expected == *value),
        _ => false,
    }
}

impl FieldPlan {
    /// The source constructor may contain an inlined delegation body. Derive
    /// missing VM-default assignments from that final reachable body, so the
    /// projection never adds a second assignment to an initialized final.
    pub fn constructor_defaults(
        &self,
        program: &Program<'_>,
        owner: usize,
        ir: &MethodIr,
    ) -> Vec<String> {
        if self.instance_defaults.is_empty() {
            return Vec::new();
        }
        let mut pending = vec![ir.entry];
        let mut reached = HashSet::new();
        let mut written = HashSet::new();
        while let Some(block) = pending.pop() {
            if !reached.insert(block) {
                continue;
            }
            let body = &ir.blocks[block];
            for statement in &body.statements {
                if let Statement::PutField {
                    field,
                    receiver: Some(receiver),
                    ..
                } = statement
                    && let Some(id) =
                        program.resolve_field(&field.owner, &field.name, &field.descriptor)
                    && id.class == owner
                    && super::constructors::is_this(ir, *receiver, &mut HashSet::new())
                {
                    written.insert(id.index);
                }
            }
            pending.extend(body.terminator.successors());
            pending.extend(body.handlers.iter().map(|handler| handler.target));
        }
        self.instance_defaults
            .iter()
            .filter(|(field, _)| !written.contains(field))
            .map(|(_, source)| source.clone())
            .collect()
    }

    pub fn new(program: &Program<'_>, types: &TypeNames<'_>) -> Result<Self> {
        let owner = types.owner;
        let class = program.class(owner);
        if class.access_flags & 0x0200 != 0 {
            return Ok(Self::default());
        }
        let candidates: BTreeSet<_> = class
            .fields
            .iter()
            .enumerate()
            .filter_map(|(id, field)| {
                (field.access_flags & 0x0010 != 0
                    && program.initialization_plan().permits_final(MemberId {
                        class: owner,
                        index: id,
                    })
                    && (field.access_flags & 0x0008 == 0 || field.constant_value.is_none()))
                .then_some(id)
            })
            .collect();
        if candidates.is_empty() {
            return Ok(Self::default());
        }
        let written: HashSet<_> = candidates
            .iter()
            .copied()
            .filter(|&index| {
                program.initialization_plan().writes_final(MemberId {
                    class: owner,
                    index,
                })
            })
            .collect();
        let mut plan = Self::default();
        // One constructor may legally leave a JVM final at its default while
        // another assigns it. A declaration initializer would conflict with the
        // latter; supply the default only in constructors which never write it.
        // This-delegating constructors are excluded by the constructor renderer.
        for (index, field) in class.fields.iter().enumerate() {
            if candidates.contains(&index)
                && field.access_flags & 0x0008 == 0
                && written.contains(&index)
            {
                let ty = JvmType::parse(&field.descriptor)?;
                plan.instance_defaults.push((
                    index,
                    format!(
                        "this.{} = {};",
                        types.names.field_name(MemberId {
                            class: owner,
                            index
                        }),
                        default_value(&ty)
                    ),
                ));
            }
        }
        let mut methods: HashSet<_> = class
            .methods
            .iter()
            .enumerate()
            .map(|(index, _)| {
                types
                    .names
                    .method_name(MemberId {
                        class: owner,
                        index,
                    })
                    .to_owned()
            })
            .collect();
        let mut helper_types = BTreeMap::new();
        for index in candidates.into_iter().filter(|id| !written.contains(id)) {
            let ty = JvmType::parse(&class.fields[index].descriptor)?;
            let initializer = if ty.is_reference() {
                "null".to_owned()
            } else {
                let helper = helper_types.entry(ty.clone()).or_insert_with(|| {
                    let mut name = format!("$default_{}", types.ty(&ty));
                    while methods.contains(&name) {
                        name.push('_');
                    }
                    methods.insert(name.clone());
                    plan.helpers.push(format!(
                        "private static {} {name}() {{ return {}; }}",
                        types.ty(&ty),
                        default_value(&ty)
                    ));
                    name
                });
                format!("{helper}()")
            };
            plan.defaults.insert(index, initializer);
        }
        Ok(plan)
    }
}
