//! The visibility of a JVM binary type and its Java declaration may differ
//! once an anonymous or local class is emitted outside its original nest.
use super::SourceUnits;
use crate::{
    classfile::{PRIVATE, PROTECTED, PUBLIC, package},
    hash::{HashMap, HashSet},
    naming::NamePlan,
    program::{ClassId, MemberId, Program},
};
use std::cell::OnceCell;

/// Actual type users, shared by placement and enclosing-instance checks. An
/// unused InnerClasses entry alone grants neither access nor an allocation site.
#[derive(Default)]
pub(in crate::java) struct TypeUsers(OnceCell<Vec<Vec<ClassId>>>);

impl TypeUsers {
    pub(in crate::java) fn of(&self, program: &Program<'_>, owner: ClassId) -> &[ClassId] {
        &self.0.get_or_init(|| {
            let mut users = vec![Vec::new(); program.own_count()];
            for id in 0..program.own_count() {
                for name in crate::java::types::used_member_types(program, id) {
                    if let Some(target) = program.id(&name).filter(|&target| program.is_own(target))
                    {
                        users[target].push(id);
                    }
                }
            }
            users
        })[owner]
    }
}

pub(in crate::java) struct MemberPath {
    pub name: String,
    root: ClassId,
    // A field on any intervening type can capture a segment in static accesses.
    pub field_shadowed: bool,
}

impl MemberPath {
    fn through(
        program: &Program<'_>,
        names: &NamePlan,
        owner: ClassId,
        prefix: Option<&Self>,
        leaf: &str,
    ) -> Self {
        let outer = prefix.map_or_else(
            || names.class_name(owner).replace('/', "."),
            |p| p.name.clone(),
        );
        let field_shadowed = prefix.is_some_and(|p| p.field_shadowed)
            || std::iter::once(owner)
                .chain(program.ancestors(owner).iter().copied())
                .any(|class| {
                    (0..program.class(class).fields.len())
                        .any(|index| names.field_name(MemberId { class, index }) == leaf)
                });
        Self {
            name: format!("{outer}.{leaf}"),
            root: prefix.map_or(owner, |path| path.root),
            field_shadowed,
        }
    }
}

impl SourceUnits {
    pub(super) fn member_paths(
        &self,
        program: &Program<'_>,
        names: &NamePlan,
        users: &TypeUsers,
        owner: ClassId,
        id: ClassId,
        suffix: &str,
    ) -> Option<(MemberPath, Option<MemberPath>)> {
        let parent = self.member(owner);
        let canonical =
            MemberPath::through(program, names, owner, parent.map(|m| &m.canonical), suffix);
        let mut exported_as = None;
        if program.class(self.lexical_root(owner)).access_flags & PUBLIC == 0 {
            exported_as = parent
                .and_then(|m| m.exported_as.as_ref())
                .map(|outer| MemberPath::through(program, names, owner, Some(outer), suffix))
                .or_else(|| {
                    self.exported_owner(program, names, owner, id, suffix)
                        .map(|outer| MemberPath::through(program, names, outer, None, suffix))
                });
            if exported_as.is_none()
                && users
                    .of(program, id)
                    .iter()
                    .any(|&user| package(names.class_name(user)) != package(names.class_name(id)))
            {
                // No legal owner spelling from another package. Preserve
                // the binary type as an independent public declaration.
                return None;
            }
        }
        Some((canonical, exported_as))
    }

    pub(super) fn reconcile_access(
        &mut self,
        program: &Program<'_>,
        names: &NamePlan,
        users: &TypeUsers,
    ) {
        // JVM access to a nested class uses its binary flags. A source member
        // additionally needs access through each enclosing declaration. Preserve
        // the authored modifiers wherever the reconstructed lexical scopes allow it.
        let mut widened = HashMap::<ClassId, u16>::default();
        for &id in self.members.keys() {
            for (target, member) in self
                .lexical_owners(id)
                .filter_map(|target| self.member(target).map(|member| (target, member)))
                .filter(|(_, member)| member.flags & PUBLIC == 0)
            {
                for &user in users.of(program, id) {
                    let same_package =
                        package(names.class_name(target)) == package(names.class_name(user));
                    if self.accessible_from(program, user, id, target, member, same_package) {
                        continue;
                    }
                    let visibility = widened.entry(target).or_default();
                    if !same_package {
                        *visibility = PUBLIC;
                    }
                }
            }
        }
        for (id, visibility) in widened {
            let flags = &mut self.members.get_mut(&id).unwrap().flags;
            *flags = (*flags & !(PRIVATE | PROTECTED)) | visibility;
        }
    }

    fn accessible_from(
        &self,
        program: &Program<'_>,
        user: ClassId,
        referenced: ClassId,
        target: ClassId,
        member: &super::Member,
        same_package: bool,
    ) -> bool {
        if self.lexical_root(user) == self.lexical_root(target) {
            return true;
        }
        if member.flags & PRIVATE != 0 {
            return false;
        }
        if same_package {
            return true;
        }
        let class = program.class(user);
        let name = &program.class(referenced).name;
        let in_header = class.super_name.as_ref() == Some(name) || class.interfaces.contains(name);
        member.flags & PROTECTED != 0
            && self
                .lexical_owners(user)
                .skip(usize::from(in_header))
                .any(|scope| program.ancestors(scope).contains(&member.owner))
    }

    // Source header dependencies include the enclosing types used to spell
    // supertypes. Adding target -> alias must not close a dependency cycle.
    fn depends_on(&self, program: &Program<'_>, alias: ClassId, target: ClassId) -> bool {
        let mut seen = HashSet::default();
        let mut pending = vec![alias];
        while let Some(id) = pending.pop() {
            if id == target {
                return true;
            }
            if !seen.insert(id) {
                continue;
            }
            pending.extend_from_slice(program.ancestors(id));
            if let Some((owner, _, _)) = program.enclosing_class(id) {
                pending.push(owner);
            }
            if let Some(exported) = self.member(id).and_then(|m| m.exported_as.as_ref()) {
                pending.push(exported.root);
            }
        }
        false
    }

    /// A public subclass can expose the member of an inaccessible declaring
    /// class. Only use a name whose inheritance path has no competing member type.
    fn exported_owner(
        &self,
        program: &Program<'_>,
        names: &NamePlan,
        owner: ClassId,
        target: ClassId,
        leaf: &str,
    ) -> Option<ClassId> {
        let has_conflict = |scope| {
            let binary = format!("{}${leaf}", names.class_name(scope));
            program.class(scope).inner_classes.iter().any(|entry| {
                entry.outer.as_deref() == Some(&program.class(scope).name)
                    && entry.inner != program.class(target).name
                    && names.class_name_for(&entry.inner) == binary
            })
        };
        program
            .descendants(owner)
            .iter()
            .copied()
            .filter(|&class| {
                class != owner
                    && program.is_own(class)
                    && program.class(class).access_flags & PUBLIC != 0
                    && program.enclosing_class(class).is_none()
                    && !self.depends_on(program, class, target)
                    && !std::iter::once(class)
                        .chain(program.ancestors(class).iter().copied())
                        .any(has_conflict)
            })
            .min_by_key(|&id| {
                let class = program.class(id);
                let original_owner = &program.class(owner).name;
                let direct = class.super_name.as_ref() == Some(original_owner)
                    || class.interfaces.contains(original_owner);
                (
                    package(names.class_name(id)) != package(names.class_name(owner)),
                    !direct,
                    names.class_name(id),
                )
            })
    }
}
