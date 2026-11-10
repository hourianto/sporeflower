use crate::hash::{HashMap, HashSet};
use crate::{
    bytecode::{self, Operand},
    classfile::{ClassFile, Constant, JvmType, MemberRef, MethodDescriptor},
};
use std::collections::{BTreeMap, BTreeSet, hash_map::Entry};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
pub(super) struct Member {
    pub name: String,
    pub descriptor: String,
    pub field: bool,
    pub is_static: Option<bool>,
}

#[derive(Default)]
pub(super) struct Requirements {
    pub classes: BTreeSet<String>,
    pub members: BTreeMap<String, BTreeSet<Member>>,
    pub interface_owners: BTreeMap<String, BTreeSet<bool>>,
    pub instantiated_classes: BTreeSet<String>,
    pub invoked_classes: BTreeSet<String>,
    pub superclasses: BTreeSet<String>,
    pub declared_interfaces: BTreeSet<String>,
    pub signature_types: BTreeSet<String>,
    pub exception_types: BTreeSet<String>,
    pub invoked_interfaces: BTreeSet<String>,
    pub floating_point: bool,
    pub warnings: Vec<String>,
}

/// Demands only accumulate in sets, so a reference or signature repeated by
/// other instructions or classes contributes nothing after its first record.
#[derive(Default)]
struct Recorded<'a> {
    references: HashSet<(&'a str, &'a str, &'a str, bool, Option<bool>)>,
    /// True once declaration or invocation evidence has recorded signature types.
    /// A constant-pool descriptor alone records only class dependencies. Reparse
    /// it once if promoted, rather than retaining every parsed type tree.
    descriptors: HashMap<&'a str, bool>,
}

impl<'a> Recorded<'a> {
    fn descriptor(&mut self, result: &mut Requirements, descriptor: &'a str, signature: bool) {
        match self.descriptors.entry(descriptor) {
            Entry::Occupied(mut entry) if signature && !*entry.get() => *entry.get_mut() = true,
            Entry::Occupied(_) => return,
            Entry::Vacant(entry) => {
                entry.insert(signature);
            }
        }
        result.descriptor_evidence(descriptor, signature);
    }

    fn reference(
        &mut self,
        result: &mut Requirements,
        member: MemberRef<'a>,
        field: bool,
        is_static: Option<bool>,
    ) {
        if !self.references.insert((
            member.owner,
            member.name,
            member.descriptor,
            field,
            is_static,
        )) {
            return;
        }
        self.descriptor(result, member.descriptor, is_static.is_some());
        result.classes.insert(member.owner.to_owned());
        result
            .members
            .entry(member.owner.to_owned())
            .or_default()
            .insert(Member {
                name: member.name.to_owned(),
                descriptor: member.descriptor.to_owned(),
                field,
                is_static,
            });
    }
}

impl Requirements {
    pub fn inspect(project: &[ClassFile]) -> Self {
        let mut result = Self::default();
        let mut recorded = Recorded::default();
        for class in project {
            result.class(class, &mut recorded);
        }
        for members in result.members.values_mut() {
            let specified: Vec<_> = members
                .iter()
                .filter(|member| member.is_static.is_some())
                .cloned()
                .collect();
            for mut member in specified {
                member.is_static = None;
                members.remove(&member);
            }
        }
        result
    }

    pub fn descriptor(&mut self, descriptor: &str) {
        self.descriptor_evidence(descriptor, false);
    }

    fn descriptor_evidence(&mut self, descriptor: &str, signature: bool) {
        if descriptor.starts_with('(') {
            if let Ok(method) = MethodDescriptor::parse(descriptor) {
                for ty in method.parameters {
                    self.ty(ty, signature);
                }
                self.ty(method.return_type, signature);
            }
        } else if let Ok(ty) = JvmType::parse(descriptor) {
            self.ty(ty, signature);
        }
    }

    /// Dependencies exposed by a library declaration. Implementation constants,
    /// instructions and handlers are deliberately excluded from API demands.
    pub fn declaration(&mut self, class: &ClassFile) {
        self.classes.extend(class.super_name.iter().cloned());
        self.classes.extend(class.interfaces.iter().cloned());
        for field in &class.fields {
            self.descriptor(&field.descriptor);
        }
        for method in &class.methods {
            self.descriptor(&method.descriptor);
            self.classes.extend(method.exceptions.iter().cloned());
        }
        for inner in &class.inner_classes {
            self.classes.insert(inner.inner.clone());
            self.classes.extend(inner.outer.iter().cloned());
        }
    }

    fn ty(&mut self, ty: JvmType, signature: bool) {
        match ty {
            JvmType::Object(name) => {
                if signature {
                    self.signature_types.insert(name.clone());
                }
                self.classes.insert(name);
            }
            JvmType::Array(element) => self.ty(*element, signature),
            JvmType::Float | JvmType::Double => self.floating_point = true,
            _ => {}
        }
    }

    fn class<'a>(&mut self, class: &'a ClassFile, recorded: &mut Recorded<'a>) {
        self.superclasses.extend(class.super_name.iter().cloned());
        self.declared_interfaces
            .extend(class.interfaces.iter().cloned());
        self.classes.extend(class.super_name.iter().cloned());
        self.classes.extend(class.interfaces.iter().cloned());
        for (index, constant) in class.constant_pool.entries.iter().enumerate() {
            match constant {
                Constant::Float(_) | Constant::Double(_) => self.floating_point = true,
                Constant::Class(_) => {
                    if let Ok(name) = class.constant_pool.class_name(index as u16) {
                        if name.starts_with('[') {
                            recorded.descriptor(self, name, false);
                        } else {
                            self.classes.insert(name.to_owned());
                        }
                    }
                }
                Constant::Fieldref { .. }
                | Constant::Methodref { .. }
                | Constant::InterfaceMethodref { .. } => {
                    if let Ok(member) = class.constant_pool.member_ref(index as u16) {
                        let field = matches!(constant, Constant::Fieldref { .. });
                        recorded.reference(self, member, field, None);
                        if !matches!(constant, Constant::Fieldref { .. }) {
                            self.interface_owners
                                .entry(member.owner.to_owned())
                                .or_default()
                                .insert(member.interface);
                        }
                    }
                }
                _ => {}
            }
        }
        for field in &class.fields {
            recorded.descriptor(self, &field.descriptor, true);
        }
        // Each distinct constant and opcode is resolved once per class.
        let mut seen = vec![0u16; class.constant_pool.entries.len()];
        let mut first_use = |index: u16, opcode: u8| {
            let Some(seen) = seen.get_mut(usize::from(index)) else {
                return false;
            };
            let bit = 1 << (opcode - 0xb2);
            let first = *seen & bit == 0;
            *seen |= bit;
            first
        };
        for method in &class.methods {
            recorded.descriptor(self, &method.descriptor, true);
            self.classes.extend(method.exceptions.iter().cloned());
            self.exception_types
                .extend(method.exceptions.iter().cloned());
            let Some(code) = &method.code else {
                continue;
            };
            self.exception_types.extend(
                code.exception_table
                    .iter()
                    .filter_map(|handler| handler.catch_type.clone()),
            );
            self.classes.extend(
                code.exception_table
                    .iter()
                    .filter_map(|handler| handler.catch_type.clone()),
            );
            let instructions = match bytecode::decode(&code.bytes) {
                Ok(instructions) => instructions,
                Err(error) => {
                    self.warnings.push(format!(
                        "API invocation-kind scan skipped {}.{}{}: {error}",
                        class.name, method.name, method.descriptor
                    ));
                    continue;
                }
            };
            for instruction in instructions {
                if instruction.opcode == 0xbb
                    && let Operand::Constant(index) = instruction.operand
                    && first_use(index, instruction.opcode)
                    && let Ok(name) = class.constant_pool.class_name(index)
                {
                    self.instantiated_classes.insert(name.to_owned());
                }
                if matches!(instruction.opcode, 0x0b..=0x0f | 0x62..=0x63 | 0x66..=0x67 | 0x6a..=0x6b | 0x6e..=0x6f | 0x72..=0x73 | 0x76..=0x77 | 0x86..=0x87 | 0x89..=0x90 | 0x95..=0x98)
                    || instruction.opcode == 0xbc
                        && matches!(instruction.operand, Operand::Int(6 | 7))
                {
                    self.floating_point = true;
                }
                if !matches!(instruction.opcode, 0xb2..=0xb9) {
                    continue;
                }
                let index = match instruction.operand {
                    Operand::Constant(index) | Operand::InterfaceCall { index, .. } => index,
                    _ => continue,
                };
                if !first_use(index, instruction.opcode) {
                    continue;
                }
                if let Ok(member) = class.constant_pool.member_ref(index) {
                    if instruction.opcode == 0xb9 || member.interface {
                        self.invoked_interfaces.insert(member.owner.to_owned());
                    } else if matches!(instruction.opcode, 0xb6..=0xb8) {
                        // A decoded class Methodref supplies class-kind evidence
                        // even for utility APIs which are never instantiated.
                        self.invoked_classes.insert(member.owner.to_owned());
                    }
                    let field = instruction.opcode <= 0xb5;
                    let is_static = matches!(instruction.opcode, 0xb2 | 0xb3 | 0xb8);
                    recorded.reference(self, member, field, Some(is_static));
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/api_requirements.rs"]
mod tests;

pub(super) fn declares(class: &ClassFile, member: &Member) -> bool {
    if member.field {
        class.fields.iter().any(|field| {
            field.name == member.name
                && field.descriptor == member.descriptor
                && member
                    .is_static
                    .is_none_or(|value| value == (field.access_flags & 8 != 0))
        })
    } else {
        class.methods.iter().any(|method| {
            method.name == member.name
                && method.descriptor == member.descriptor
                && member
                    .is_static
                    .is_none_or(|value| value == (method.access_flags & 8 != 0))
        })
    }
}
