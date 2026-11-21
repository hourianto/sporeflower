//! Java compilation units keep binary identities for mappings and reflection.
//! Only reciprocal InnerClasses records establish a member declaration. Local
//! and anonymous bodies retain explicit constructor/capture state as package
//! companions when their allocating owner is known.
mod access;
pub(super) use access::{MemberPath, TypeUsers};

use super::{DecompiledClass, JavaOptions, body::CodeWriter};
use crate::{
    Result,
    classfile::{Constant, PRIVATE, PROTECTED, PUBLIC, STATIC, package},
    hash::{HashMap, HashSet},
    naming::{NamePlan, is_identifier, simple_name},
    program::{ClassId, MemberId, Program},
};
use std::collections::BTreeSet;

pub(super) struct Member {
    pub owner: ClassId,
    pub simple: String,
    pub canonical: MemberPath,
    // Inherited names can expose a member whose declaring class is hidden.
    pub exported_as: Option<MemberPath>,
    pub flags: u16,
    pub capture: Option<MemberId>,
}

impl Member {
    pub(super) fn path(&self, same_package: bool) -> &MemberPath {
        if !same_package && let Some(exported) = &self.exported_as {
            return exported;
        }
        &self.canonical
    }
}

#[derive(Default)]
pub struct SourceUnits {
    members: HashMap<ClassId, Member>,
    qualification_guards: HashMap<MemberId, HashMap<u32, u32>>,
    roots: HashMap<ClassId, ClassId>,
    groups: HashMap<ClassId, Vec<ClassId>>,
}

impl SourceUnits {
    pub fn new(program: &Program<'_>, names: &NamePlan) -> Self {
        let mut units = Self::default();
        // A binary member name extends its owner's name, even when mappings
        // rename both. Processing shorter names first also rules out cycles.
        let mut order: Vec<_> = (0..program.own_count())
            .filter(|&id| !program.class(id).inner_classes.is_empty())
            .collect();
        if order.is_empty() {
            return units;
        }
        order.sort_by_key(|&id| (names.class_name(id).len(), id));
        let package_roots = names
            .class_mappings()
            .filter_map(|(_, emitted)| emitted.split_once('/').map(|(root, _)| root))
            .chain(["java"])
            .collect();
        let users = TypeUsers::default();
        for id in order {
            units.place(program, names, id, &package_roots, &users);
        }
        units.reconcile_access(program, names, &users);
        units.separate_referenced_companions(program, &users);
        units
    }

    /// The outer class owns the file. Named Java members also own a lexical
    /// scope; package companions intentionally keep their binary type spelling.
    fn place(
        &mut self,
        program: &Program<'_>,
        names: &NamePlan,
        id: ClassId,
        package_roots: &HashSet<&str>,
        users: &TypeUsers,
    ) -> Option<()> {
        let class = program.class(id);
        let (owner, entry, _) = program.enclosing_class(id)?;
        let parent = program.class(owner);
        let suffix = names
            .class_name(id)
            .strip_prefix(names.class_name(owner))?
            .strip_prefix('$')?;
        if super::linkage::validate_class(program, owner).is_err()
            || super::linkage::validate_class(program, id).is_err()
        {
            return None;
        }
        let named = entry.outer.is_some() && entry.inner_name.is_some();
        if named
            && (self.capture(owner).is_none() || entry.access_flags & STATIC == 0)
            && is_identifier(suffix)
            && !package_roots.contains(suffix)
            && !self
                .lexical_owners(owner)
                .any(|outer| self.simple(names, outer) == suffix)
            && !(package(names.class_name(id)).is_empty()
                && names.visible_package_types(id)[0].contains(suffix))
        {
            let capture = if entry.access_flags & STATIC == 0 {
                super::enclosing::capture(program, names, id, owner, users)
            } else {
                None
            };
            if entry.access_flags & STATIC != 0 || capture.is_some() {
                let (canonical, exported_as) =
                    self.member_paths(program, names, users, owner, id, suffix)?;
                let field = capture.map(|capture| {
                    for (method, guard, allocation) in capture.guards {
                        self.qualification_guards
                            .entry(method)
                            .or_default()
                            .insert(guard, allocation);
                    }
                    capture.field
                });
                self.members.insert(
                    id,
                    Member {
                        owner,
                        simple: suffix.to_owned(),
                        canonical,
                        exported_as,
                        flags: entry.access_flags,
                        capture: field,
                    },
                );
            }
        }
        if class.access_flags & PUBLIC != 0 && !self.members.contains_key(&id) {
            return None;
        }
        // Without a named outer entry, require a constructor reference in the
        // enclosing class in addition to matching binary prefix and metadata.
        if entry.outer.is_none()
            && !parent
                .constant_pool
                .entries
                .iter()
                .enumerate()
                .any(|(i, c)| {
                    matches!(c, Constant::Methodref { .. })
                        && parent
                            .constant_pool
                            .member_ref(i as u16)
                            .is_ok_and(|m| m.owner == class.name && m.name == "<init>")
                })
        {
            return None;
        }
        let root = self.root(owner);
        self.roots.insert(id, root);
        self.groups.entry(root).or_default().push(id);
        Some(())
    }

    // javac -sourcepath finds a top-level type by its own filename. Split a
    // companion only when a different file needs it, propagating that decision
    // through other companions it uses.
    fn separate_referenced_companions(&mut self, program: &Program<'_>, users: &TypeUsers) {
        let mut dependencies = HashMap::<ClassId, Vec<ClassId>>::default();
        let mut separate = HashSet::default();
        let mut pending = Vec::new();
        for &id in self.roots.keys() {
            let target = self.lexical_root(id);
            if target == self.root(target) {
                continue;
            }
            for &user in users.of(program, id) {
                let source = self.lexical_root(user);
                if source == target {
                    continue;
                }
                if self.root(source) != self.root(target) {
                    if separate.insert(target) {
                        pending.push(target);
                    }
                } else {
                    dependencies.entry(source).or_default().push(target);
                }
            }
        }
        while let Some(source) = pending.pop() {
            for &target in dependencies.get(&source).into_iter().flatten() {
                if separate.insert(target) {
                    pending.push(target);
                }
            }
        }
        if separate.is_empty() {
            return;
        }
        for (root, group) in std::mem::take(&mut self.groups) {
            for id in group {
                let owner = self.lexical_root(id);
                if separate.contains(&owner) {
                    if owner == id {
                        self.roots.remove(&id);
                    } else {
                        self.roots.insert(id, owner);
                        self.groups.entry(owner).or_default().push(id);
                    }
                } else {
                    self.groups.entry(root).or_default().push(id);
                }
            }
        }
    }

    /// A source filename uses the emitted binary name of the unit's root.
    pub fn root(&self, id: ClassId) -> ClassId {
        self.roots.get(&id).copied().unwrap_or(id)
    }

    pub(super) fn member(&self, id: ClassId) -> Option<&Member> {
        self.members.get(&id)
    }

    pub(super) fn importable(&self, program: &Program<'_>, id: ClassId) -> bool {
        self.lexical_owners(id).all(|id| {
            self.member(id)
                .map_or_else(|| program.class(id).access_flags, |m| m.flags)
                & PUBLIC
                != 0
        })
    }

    pub(super) fn capture(&self, id: ClassId) -> Option<MemberId> {
        self.members.get(&id)?.capture
    }

    pub(super) fn qualification_guards(&self, member: MemberId) -> Option<&HashMap<u32, u32>> {
        self.qualification_guards.get(&member)
    }

    fn lexical_owners(&self, id: ClassId) -> impl Iterator<Item = ClassId> + '_ {
        std::iter::successors(Some(id), |id| self.member(*id).map(|m| m.owner))
    }

    fn lexical_root(&self, id: ClassId) -> ClassId {
        self.lexical_owners(id).last().unwrap()
    }

    pub(super) fn scopes<'a>(
        &'a self,
        program: &'a Program<'_>,
        id: ClassId,
    ) -> impl Iterator<Item = ClassId> + 'a {
        self.lexical_owners(id)
            .flat_map(|id| std::iter::once(id).chain(program.ancestors(id).iter().copied()))
    }

    pub(super) fn visible_members(
        &self,
        program: &Program<'_>,
        scope: ClassId,
        from: ClassId,
        same_package: bool,
        visible: &mut HashMap<String, Option<ClassId>>,
    ) {
        // Inherited member names can be hidden even by an inaccessible type
        // along an intermediate superclass. Reserve them against imports and
        // use the declaring owner for binding. Lexical declarations bind directly.
        let lexical = self.lexical_owners(from).any(|id| id == scope);
        for entry in &program.class(scope).inner_classes {
            if let Some(id) = program.id(&entry.inner)
                && let Some(member) = self.members.get(&id)
                && member.owner == scope
                && (member.flags & (PUBLIC | PROTECTED) != 0
                    || member.flags & PRIVATE == 0 && same_package
                    || lexical)
            {
                let binding = lexical.then_some(id);
                visible
                    .entry(member.simple.clone())
                    .and_modify(|previous| {
                        if *previous != binding {
                            *previous = None;
                        }
                    })
                    .or_insert(binding);
            }
        }
    }

    pub(super) fn simple<'a>(&'a self, names: &'a NamePlan, id: ClassId) -> &'a str {
        self.members.get(&id).map_or_else(
            || simple_name(names.class_name(id)),
            |member| &member.simple,
        )
    }

    pub(super) fn standalone(
        &self,
        program: &Program<'_>,
        names: &NamePlan,
        id: ClassId,
        options: &JavaOptions<'_>,
    ) -> Result<DecompiledClass> {
        let class = super::Declaration::new(program, names, self, id, options)?;
        let (mut body, _) = super::declaration(program, options, &class, &class.types)?;
        let mut file = file_header(names.class_name(id), &class.types.take_imports());
        file.text.push_str(&body.source);
        file.line("}");
        body.source = file.text;
        Ok(body)
    }

    /// Emit the containing unit for any original member identity.
    pub fn decompile(
        &self,
        program: &Program<'_>,
        names: &NamePlan,
        id: ClassId,
        options: &JavaOptions<'_>,
    ) -> Result<DecompiledClass> {
        let root = self.root(id);
        anyhow::ensure!(program.is_own(id), "class is not an application class");
        let Some(group) = self.groups.get(&root) else {
            return Ok(self
                .standalone(program, names, id, options)
                .unwrap_or_else(|error| {
                    let mut result = DecompiledClass::default();
                    result.record_failure(&program.class(id).name, error);
                    result
                }));
        };
        let mut result = DecompiledClass::default();
        let mut classes = Vec::with_capacity(1 + group.len());
        for id in std::iter::once(root).chain(group.iter().copied()) {
            match super::Declaration::new(program, names, self, id, options) {
                Ok(class) => classes.push(class),
                Err(error) => result.record_failure(&program.class(id).name, error),
            }
        }
        super::types::reconcile_unit(&mut classes);
        // Parents precede their members in the placement plan. Only attach a
        // declaration after it renders, so a failed parent excludes its subtree.
        let mut declarations = HashMap::<ClassId, EmittedDeclaration>::default();
        let mut top_level = Vec::new();
        for (index, class) in classes.iter().enumerate() {
            let id = class.types.owner;
            let member = self.member(id);
            if let Some(member) = member
                && !declarations.contains_key(&member.owner)
            {
                result.record_failure(
                    &program.class(id).name,
                    anyhow::anyhow!(
                        "enclosing class {} failed",
                        program.class(member.owner).name
                    ),
                );
                continue;
            }
            // Java 1.3 allows inner classes at any depth. Their static helpers
            // need both the declaration and the type bindings of a static host.
            let host = member.filter(|m| m.capture.is_some()).map(|m| {
                self.lexical_owners(m.owner)
                    .find(|&id| self.capture(id).is_none())
                    .unwrap()
            });
            let support_types =
                host.map_or(&class.types, |id| &classes[declarations[&id].index].types);
            let (body, support) = match super::declaration(program, options, class, support_types) {
                Ok(body) => body,
                Err(error) => {
                    result.record_failure(&program.class(id).name, error);
                    continue;
                }
            };
            if let Some(member) = member {
                declarations
                    .get_mut(&member.owner)
                    .unwrap()
                    .children
                    .push(id);
                if let Some(host) = host {
                    declarations.get_mut(&host).unwrap().text.push_str(&support);
                }
            } else {
                top_level.push(id);
            }
            declarations.insert(
                id,
                EmittedDeclaration {
                    index,
                    text: body.source,
                    children: Vec::new(),
                },
            );
            result.total_methods += body.total_methods;
            result.failed_methods += body.failed_methods;
            result.fallback_methods += body.fallback_methods;
            result.inspection_fallback_methods += body.inspection_fallback_methods;
            result.simplifications += body.simplifications;
            result.diagnostics.extend(body.diagnostics);
        }
        if declarations.is_empty() {
            return Ok(result);
        }
        let imports = declarations
            .values()
            .flat_map(|d| classes[d.index].types.take_imports())
            .collect();
        let mut file = file_header(names.class_name(root), &imports);
        for (index, id) in top_level.into_iter().enumerate() {
            if index > 0 {
                file.line("");
            }
            declarations[&id].write(&declarations, &mut file);
        }
        result.source = file.text;
        Ok(result)
    }
}

struct EmittedDeclaration {
    index: usize,
    text: String,
    children: Vec<ClassId>,
}

impl EmittedDeclaration {
    fn write(&self, declarations: &HashMap<ClassId, Self>, out: &mut CodeWriter) {
        if out.indent == 0 {
            out.text.push_str(&self.text);
        } else {
            out.line(self.text.trim_end_matches('\n'));
        }
        out.indent += 1;
        for child in &self.children {
            out.line("");
            declarations[child].write(declarations, out);
        }
        out.close("}");
    }
}

fn file_header(binary_name: &str, imports: &BTreeSet<String>) -> CodeWriter {
    let mut file = CodeWriter::new(0);
    let pkg = package(binary_name);
    if !pkg.is_empty() {
        file.line(format!("package {};", pkg.replace('/', ".")));
        file.line("");
    }
    for import in imports {
        file.line(format!("import {import};"));
    }
    if !imports.is_empty() {
        file.line("");
    }
    file
}
