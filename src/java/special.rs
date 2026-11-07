//! A nonprivate INVOKESPECIAL to a current-class method cannot be written as
//! an ordinary invocation when a subtype overrides it. Split only those actual
//! targets into a virtual entry and an exact, private implementation entry.
use super::types::TypeNames;
use crate::Result;
use crate::bytecode::Operand;
use crate::classfile::{Method, MethodDescriptor};
use crate::{
    naming::NamePlan,
    program::{ClassId, MemberId, Program},
};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;

#[derive(Clone, Default)]
pub struct SpecialPlan {
    helpers: HashMap<MemberId, String>,
}

impl SpecialPlan {
    pub fn new(program: &Program<'_>, names: &NamePlan, owner: ClassId) -> Self {
        let class = program.class(owner);
        let mut targets = BTreeSet::new();
        for method in &class.methods {
            let Some(code) = &method.code else {
                continue;
            };
            if !code.bytes.contains(&0xb7) {
                continue;
            }
            let Ok(instructions) = crate::bytecode::decode(&code.bytes) else {
                continue;
            };
            for instruction in instructions {
                if instruction.opcode != 0xb7 {
                    continue;
                }
                let Operand::Constant(index) = instruction.operand else {
                    continue;
                };
                let Ok(reference) = class.constant_pool.member_ref(index) else {
                    continue;
                };
                if reference.name.starts_with('<') {
                    continue;
                }
                let Some(target) =
                    program.resolve_method(reference.owner, reference.name, reference.descriptor)
                else {
                    continue;
                };
                if target.class != owner {
                    continue;
                }
                let declaration = program.method(target);
                if declaration.code.is_none()
                    || declaration.access_flags & (0x0002 | 0x0008 | 0x0010) != 0
                {
                    continue;
                }
                let overridden = (0..program.own_count()).any(|subclass| {
                    subclass != owner
                        && program.ancestors(subclass).contains(&owner)
                        && program.visible_from(declaration.access_flags, owner, subclass)
                        && program
                            .declared_method(subclass, &declaration.name, &declaration.descriptor)
                            .is_some_and(|method| {
                                program.method(method).access_flags & (0x0002 | 0x0008) == 0
                            })
                });
                if overridden {
                    targets.insert(target);
                }
            }
        }
        if targets.is_empty() {
            return Self::default();
        }
        let mut occupied: HashSet<_> = std::iter::once(owner)
            .chain(program.ancestors(owner).iter().copied())
            .flat_map(|class| {
                program
                    .class(class)
                    .methods
                    .iter()
                    .enumerate()
                    .map(move |(index, _)| names.method_name(MemberId { class, index }).to_owned())
            })
            .collect();
        let mut helpers = HashMap::new();
        for target in targets {
            let base = format!("$sf$special_{}", target.index);
            let mut candidate = base.clone();
            let mut suffix = 0;
            while !occupied.insert(candidate.clone()) {
                candidate = format!("{base}_{suffix}");
                suffix += 1;
            }
            helpers.insert(target, candidate);
        }
        Self { helpers }
    }

    pub fn helper(&self, method: MemberId) -> Option<&str> {
        self.helpers.get(&method).map(String::as_str)
    }
}

pub fn helper_header(
    types: &TypeNames<'_>,
    method: &Method,
    name: &str,
    exceptions: &[String],
    parameter_names: &[String],
) -> Result<String> {
    let descriptor = MethodDescriptor::parse(&method.descriptor)?;
    let mut header = String::from("private ");
    if method.access_flags & 0x0020 != 0 {
        header.push_str("synchronized ");
    }
    if method.access_flags & 0x0800 != 0 {
        header.push_str("strictfp ");
    }
    header.push_str(&types.ty(&descriptor.return_type));
    header.push(' ');
    header.push_str(name);
    header.push('(');
    for (index, parameter) in descriptor.parameters.iter().enumerate() {
        if index > 0 {
            header.push_str(", ");
        }
        write!(header, "{} {}", types.ty(parameter), parameter_names[index]).unwrap();
    }
    header.push(')');
    if !exceptions.is_empty() {
        header.push_str(" throws ");
        header.push_str(
            &exceptions
                .iter()
                .map(|name| types.class(name))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    Ok(header)
}

pub fn forwarding_body(
    method: &Method,
    helper: &str,
    parameter_names: &[String],
) -> Result<String> {
    let descriptor = MethodDescriptor::parse(&method.descriptor)?;
    let parameters = parameter_names.join(", ");
    let prefix = if descriptor.return_type == crate::classfile::JvmType::Void {
        ""
    } else {
        "return "
    };
    Ok(format!("{prefix}this.{helper}({parameters});"))
}
