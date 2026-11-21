//! Java compilation units keep binary identities for mappings and reflection.
//! Only reciprocal InnerClasses records establish a member declaration. Local
//! and anonymous bodies retain explicit constructor/capture state as package
//! companions when their allocating owner is known.
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
    pub canonical: String,
    pub flags: u16,
    pub capture: Option<MemberId>,
}

#[derive(Default)]
pub struct SourceUnits {
    members: HashMap<ClassId, Member>,
    qualification_guards: HashMap<MemberId, HashSet<u32>>,
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
        let users = super::enclosing::Users::default();
        for id in order {
            units.place(program, names, id, &package_roots, &users);
        }
        units.reconcile_access(program, names);
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
        users: &super::enclosing::Users,
    ) -> Option<()> {
        let class = program.class(id);
        let mut own = class
            .inner_classes
            .iter()
            .filter(|entry| entry.inner == class.name);
        let entry = own.next()?;
        if own.next().is_some() {
            return None;
        }
        let outer = entry
            .outer
            .as_deref()
            .or_else(|| class.name.rsplit_once('$').map(|(prefix, _)| prefix))?;
        let owner = program.id(outer).filter(|&id| program.is_own(id))?;
        let parent = program.class(owner);
        let suffix = names
            .class_name(id)
            .strip_prefix(names.class_name(owner))?
            .strip_prefix('$')?;
        if !parent.inner_classes.contains(entry)
            || super::linkage::validate_class(program, owner).is_err()
            || super::linkage::validate_class(program, id).is_err()
        {
            return None;
        }
        let named = entry.outer.is_some() && entry.inner_name.is_some();
        if named
            && class.name.strip_prefix(&parent.name)?.strip_prefix('$')
                != entry.inner_name.as_deref()
        {
            return None;
        }
        if named
            && self.capture(owner).is_none()
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
                let outer = self
                    .member(owner)
                    .map(|member| member.canonical.clone())
                    .unwrap_or_else(|| names.class_name(owner).replace('/', "."));
                let field = capture.map(|capture| {
                    for (method, pc) in capture.guards {
                        self.qualification_guards
                            .entry(method)
                            .or_default()
                            .insert(pc);
                    }
                    capture.field
                });
                self.members.insert(
                    id,
                    Member {
                        owner,
                        simple: suffix.to_owned(),
                        canonical: format!("{outer}.{suffix}"),
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

    fn reconcile_access(&mut self, program: &Program<'_>, names: &NamePlan) {
        let packages: HashSet<_> = self
            .members
            .iter()
            .filter(|(_, member)| member.flags & PRIVATE != 0)
            .map(|(&id, _)| package(names.class_name(id)))
            .collect();
        if packages.is_empty() {
            return;
        }

        // A package companion can refer to a private member of its former nest.
        // Widen only declarations named by actual code, signatures or debug locals.
        let mut widen = HashSet::default();
        for id in 0..program.own_count() {
            let pkg = package(names.class_name(id));
            if !packages.contains(pkg) {
                continue;
            }
            let scope = self.lexical_owners(id).last();
            for reference in super::types::used_types(program.class(id)) {
                let Some(target) = program.id(&reference) else {
                    continue;
                };
                if self.members.contains_key(&target)
                    && package(names.class_name(target)) == pkg
                    && self.lexical_owners(target).last() != scope
                {
                    widen.extend(
                        self.lexical_owners(target)
                            .filter(|id| self.member(*id).is_some_and(|m| m.flags & PRIVATE != 0)),
                    );
                }
            }
        }
        for id in widen {
            self.members.get_mut(&id).unwrap().flags &= !PRIVATE;
        }
    }

    /// A source filename uses the emitted binary name of the unit's root.
    pub fn root(&self, id: ClassId) -> ClassId {
        self.roots.get(&id).copied().unwrap_or(id)
    }

    pub(super) fn member(&self, id: ClassId) -> Option<&Member> {
        self.members.get(&id)
    }

    pub(super) fn capture(&self, id: ClassId) -> Option<MemberId> {
        self.members.get(&id)?.capture
    }

    pub(super) fn qualification_guards(&self, member: MemberId) -> Option<&HashSet<u32>> {
        self.qualification_guards.get(&member)
    }

    fn lexical_owners(&self, id: ClassId) -> impl Iterator<Item = ClassId> + '_ {
        std::iter::successors(Some(id), |id| self.member(*id).map(|m| m.owner))
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
        for entry in &program.class(scope).inner_classes {
            if let Some(id) = program.id(&entry.inner)
                && let Some(member) = self.members.get(&id)
                && member.owner == scope
                && (member.flags & (PUBLIC | PROTECTED) != 0
                    || member.flags & PRIVATE == 0 && same_package
                    || self.lexical_owners(from).last() == self.lexical_owners(scope).last())
            {
                visible
                    .entry(member.simple.clone())
                    .and_modify(|previous| {
                        if *previous != Some(id) {
                            *previous = None;
                        }
                    })
                    .or_insert(Some(id));
            }
        }
    }

    pub(super) fn simple<'a>(&'a self, names: &'a NamePlan, id: ClassId) -> &'a str {
        self.members.get(&id).map_or_else(
            || simple_name(names.class_name(id)),
            |member| &member.simple,
        )
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
        let Some(group) = self.groups.get(&root) else {
            let class = super::Declaration::new(program, names, self, root, options)?;
            let mut body = super::declaration(program, options, &class)?;
            let mut file = file_header(names.class_name(root), &class.types.take_imports());
            file.text.push_str(&body.source);
            file.line("}");
            body.source = file.text;
            return Ok(body);
        };
        let mut result = DecompiledClass::default();
        let mut classes = Vec::with_capacity(1 + group.len());
        let record_failure = |result: &mut DecompiledClass, id, error: anyhow::Error| {
            result.failed_classes += 1;
            result
                .diagnostics
                .push(format!("{}: {error:#}", program.class(id).name));
        };
        for id in std::iter::once(root).chain(group.iter().copied()) {
            match super::Declaration::new(program, names, self, id, options) {
                Ok(class) => classes.push(class),
                Err(error) => record_failure(&mut result, id, error),
            }
        }
        super::types::reconcile_unit(&mut classes);
        let mut imports = BTreeSet::new();
        // Parents precede their members in the placement plan. Only attach a
        // declaration after it renders, so a failed parent excludes its subtree.
        let mut declarations = HashMap::<ClassId, EmittedDeclaration>::default();
        let mut top_level = Vec::new();
        for class in classes {
            let id = class.types.owner;
            let siblings = if let Some(member) = self.member(id) {
                let Some(parent) = declarations.get_mut(&member.owner) else {
                    record_failure(
                        &mut result,
                        id,
                        anyhow::anyhow!(
                            "enclosing class {} failed",
                            program.class(member.owner).name
                        ),
                    );
                    continue;
                };
                &mut parent.children
            } else {
                &mut top_level
            };
            let body = match super::declaration(program, options, &class) {
                Ok(body) => body,
                Err(error) => {
                    record_failure(&mut result, id, error);
                    continue;
                }
            };
            imports.extend(class.types.take_imports());
            siblings.push(id);
            declarations.insert(
                id,
                EmittedDeclaration {
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
