//! Java declarations for dispatch contracts which classfiles need not spell out.
//! Names and lookup stay tied to the original JVM member identities.

use crate::Result;
use crate::classfile::MethodDescriptor;
use crate::{
    naming::NamePlan,
    program::{ClassId, MemberId, Program},
};
use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::{body::CodeWriter, types::TypeNames};

const PUBLIC: u16 = 0x0001;
const PRIVATE: u16 = 0x0002;
const PROTECTED: u16 = 0x0004;
const STATIC: u16 = 0x0008;
const ABSTRACT: u16 = 0x0400;
const INTERFACE: u16 = 0x0200;

/// Source visibility must cover every inherited declaration in an override
/// family. The JVM permits narrowed access on an otherwise valid override.
pub(super) fn method_flags(program: &Program<'_>, member: MemberId) -> u16 {
    let method = program.method(member);
    if method.name.starts_with('<') || method.access_flags & (PRIVATE | STATIC) != 0 {
        return method.access_flags;
    }
    let mut access = access_rank(method.access_flags);
    for &ancestor in program.ancestors(member.class) {
        let Some(inherited) = program.declared_method(ancestor, &method.name, &method.descriptor)
        else {
            continue;
        };
        let inherited = program.method(inherited);
        if inherited.access_flags & (PRIVATE | STATIC) == 0
            && program.visible_from(inherited.access_flags, ancestor, member.class)
        {
            access = access.max(access_rank(inherited.access_flags));
        }
    }
    (method.access_flags & !(PUBLIC | PRIVATE | PROTECTED)) | access_flags(access)
}

pub(super) struct MissingMethod {
    pub contract: MemberId,
    access: u16,
    exceptions: Vec<String>,
}

pub(super) struct Contracts {
    pub missing: Vec<MissingMethod>,
}

impl Contracts {
    pub fn uses_cldc_error(&self, program: &Program<'_>) -> bool {
        !self.missing.is_empty()
            && program.id("java/lang/AbstractMethodError").is_none()
            && program.id("java/lang/Error").is_some()
    }

    pub fn build(program: &Program<'_>, owner: ClassId) -> Self {
        let class = program.class(owner);
        if class.access_flags & (ABSTRACT | INTERFACE) != 0 {
            return Self {
                missing: Vec::new(),
            };
        }
        let mut required = BTreeMap::<(&str, &str), Vec<MemberId>>::new();
        for &ancestor in program.ancestors(owner) {
            for (index, method) in program.class(ancestor).methods.iter().enumerate() {
                if method.access_flags & ABSTRACT != 0
                    && method.access_flags & (PRIVATE | STATIC) == 0
                    && program.visible_from(method.access_flags, ancestor, owner)
                {
                    required
                        .entry((&method.name, &method.descriptor))
                        .or_default()
                        .push(MemberId {
                            class: ancestor,
                            index,
                        });
                }
            }
        }
        let mut missing = Vec::new();
        for ((name, descriptor), declarations) in required {
            // Class lookup wins over interface declarations. A real concrete
            // body must never be replaced merely because an interface repeats
            // its signature.
            if class_implementation(program, owner, name, descriptor).is_some() {
                continue;
            }
            // A concrete superclass receives this same source declaration. Its
            // descendants inherit it unless an intervening abstract override
            // deliberately restores the missing contract.
            if inherits_generated_method(program, owner, name, descriptor) {
                continue;
            }
            let access = declarations
                .iter()
                .map(|&id| access_rank(program.method(id).access_flags))
                .max()
                .unwrap_or(0);
            let mut exceptions: Vec<_> = declarations
                .iter()
                .flat_map(|&id| program.method_throws(id).iter().cloned())
                .filter(|exception| {
                    declarations.iter().all(|&id| {
                        program
                            .method_throws(id)
                            .iter()
                            .any(|allowed| program.is_subtype(exception, allowed))
                    })
                })
                .collect();
            exceptions.sort();
            exceptions.dedup();
            missing.push(MissingMethod {
                contract: declarations[0],
                access: access_flags(access),
                exceptions,
            });
        }
        Self { missing }
    }

    pub fn emit(
        &self,
        program: &Program<'_>,
        names: &NamePlan,
        types: &TypeNames<'_>,
        out: &mut CodeWriter,
    ) -> Result<()> {
        for missing in &self.missing {
            let method = program.method(missing.contract);
            let descriptor = MethodDescriptor::parse(&method.descriptor)?;
            let mut header = match missing.access {
                PUBLIC => "public ".to_owned(),
                PROTECTED => "protected ".to_owned(),
                _ => String::new(),
            };
            header.push_str(&types.ty(&descriptor.return_type));
            header.push(' ');
            header.push_str(names.method_name(missing.contract));
            header.push('(');
            for (index, parameter) in descriptor.parameters.iter().enumerate() {
                if index != 0 {
                    header.push_str(", ");
                }
                write!(header, "{} p{}", types.ty(parameter), index + 1).unwrap();
            }
            header.push(')');
            header.push_str(&super::generated::throws_clause(&missing.exceptions, types));
            out.line("");
            out.open(format!("{header} {{"));
            if self.uses_cldc_error(program) {
                // CLDC 1.1 §4.2 permits the nearest supported Error superclass
                // when a VM error subtype is absent from the limited library.
                out.line("// CLDC exposes this missing method's VM error as java.lang.Error.");
                out.line("throw new java.lang.Error();");
            } else {
                out.line("throw new java.lang.AbstractMethodError();");
            }
            out.close("}");
        }
        Ok(())
    }
}

fn class_implementation(
    program: &Program<'_>,
    owner: ClassId,
    name: &str,
    descriptor: &str,
) -> Option<MemberId> {
    let mut current = Some(owner);
    while let Some(class) = current {
        if let Some(member) = program.declared_method(class, name, descriptor) {
            let method = program.method(member);
            if method.access_flags & (PRIVATE | STATIC) == 0
                && program.visible_from(method.access_flags, class, owner)
            {
                return (method.access_flags & ABSTRACT == 0).then_some(member);
            }
        }
        current = program
            .class(class)
            .super_name
            .as_deref()
            .and_then(|name| program.id(name));
    }
    None
}

fn inherits_generated_method(
    program: &Program<'_>,
    owner: ClassId,
    name: &str,
    descriptor: &str,
) -> bool {
    let mut current = program
        .class(owner)
        .super_name
        .as_deref()
        .and_then(|name| program.id(name));
    while let Some(class) = current {
        if program.declared_method(class, name, descriptor).is_some() {
            return false;
        }
        if program.is_own(class) && program.class(class).access_flags & (ABSTRACT | INTERFACE) == 0
        {
            return program.ancestors(class).iter().any(|&ancestor| {
                program
                    .declared_method(ancestor, name, descriptor)
                    .is_some_and(|member| {
                        let method = program.method(member);
                        method.access_flags & ABSTRACT != 0
                            && method.access_flags & (PRIVATE | STATIC) == 0
                            && program.visible_from(method.access_flags, ancestor, class)
                    })
            });
        }
        current = program
            .class(class)
            .super_name
            .as_deref()
            .and_then(|name| program.id(name));
    }
    false
}

fn access_rank(flags: u16) -> u8 {
    if flags & PUBLIC != 0 {
        3
    } else if flags & PROTECTED != 0 {
        2
    } else {
        1
    }
}

fn access_flags(rank: u8) -> u16 {
    match rank {
        3 => PUBLIC,
        2 => PROTECTED,
        _ => 0,
    }
}
