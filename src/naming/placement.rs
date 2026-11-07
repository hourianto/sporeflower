//! Authored placement uses Java identities for libraries and portable paths for
//! emitted files. Package boundaries are checked against the original program,
//! rather than prohibiting every split or merge of an input package.

use super::*;
use crate::bytecode::{Operand, decode};

pub(super) struct Placement {
    pub classes: Vec<String>,
    pub check_boundaries: bool,
}

pub(super) fn allocate(
    program: &Program<'_>,
    options: &RenameOptions,
    overrides: &NameOverrides,
    origins: &MappingOrigins,
    diagnostics: &mut Vec<String>,
) -> Result<Placement> {
    let automatic = allocate_classes(program, options, diagnostics);
    if overrides.classes.is_empty() && overrides.reserved_classes.is_empty() {
        return Ok(Placement {
            classes: automatic,
            check_boundaries: false,
        });
    }

    let mut requested = BTreeMap::<&str, BTreeSet<&str>>::new();
    for (&id, name) in &overrides.classes {
        if program.is_own(id) {
            requested
                .entry(package(&program.class(id).name))
                .or_default()
                .insert(package(name));
        }
    }
    // Preserve the established convenience of moving unmapped package peers
    // when there is one destination. A split has no unique implied destination.
    let mut groups = BTreeMap::<&str, String>::new();
    for (id, class) in program.classes()[..program.own_count()].iter().enumerate() {
        if !overrides.classes.contains_key(&id) {
            let original = package(&class.name);
            let destination = requested
                .get(original)
                .filter(|destinations| destinations.len() == 1)
                .and_then(|destinations| destinations.first().copied())
                .unwrap_or_else(|| package(&automatic[id]));
            groups.insert(original, destination.to_owned());
        }
    }

    let mut namespace = Namespace::default();
    for class in &program.classes()[program.own_count()..] {
        namespace.external_types.insert(class.name.clone());
        namespace
            .external_packages
            .extend(package_prefixes(package(&class.name)).map(str::to_owned));
    }
    let mut fixed: Vec<_> = overrides
        .classes
        .iter()
        .filter(|(id, _)| program.is_own(**id))
        .map(|(_, name)| name.as_str())
        .chain(overrides.reserved_classes.iter().map(String::as_str))
        .collect();
    fixed.sort_unstable();
    for &name in &fixed {
        ensure!(
            !namespace.external_types.contains(name),
            placement_error(
                format!("authored or source-only class {name} collides with an external class"),
                &[name],
                overrides,
                origins
            )
        );
        if let Some(previous) = namespace.types.insert(path_key(name), name.to_owned()) {
            bail!(placement_error(
                format!("authored or source-only class name {name} collides with {previous}"),
                &[name, &previous],
                overrides,
                origins
            ));
        }
    }
    for &name in &fixed {
        namespace.reserve_package(package(name), overrides, origins)?;
    }
    let reserved_packages: HashSet<_> = groups.values().map(|name| path_key(name)).collect();
    let mut counter = 0;
    for (original, destination) in &mut groups {
        while let Some(conflict) = namespace.package_conflict(destination) {
            ensure!(
                !requested.contains_key(original),
                placement_error(
                    format!(
                        "authored package {original} -> {destination} collides with {conflict}"
                    ),
                    &[destination, conflict],
                    overrides,
                    origins
                )
            );
            loop {
                *destination = format!("package_{counter}");
                counter += 1;
                if !reserved_packages.contains(&path_key(destination)) {
                    break;
                }
            }
        }
        namespace.reserve_package(destination, overrides, origins)?;
    }
    let roots: HashSet<_> = namespace
        .packages
        .values()
        .chain(&namespace.external_packages)
        .map(|name| name.split('/').next().unwrap().to_owned())
        .collect();
    for &name in &fixed {
        ensure!(
            !namespace.external_packages.contains(name)
                && !namespace.packages.contains_key(&path_key(name))
                && !roots.contains(simple_name(name)),
            placement_error(
                format!("authored or source-only class {name} conflicts with a package name"),
                &[name],
                overrides,
                origins
            )
        );
    }

    let mut names = automatic.clone();
    for owner in 0..program.own_count() {
        names[owner] = overrides.classes.get(&owner).cloned().unwrap_or_else(|| {
            join(
                &groups[package(&program.class(owner).name)],
                simple_name(&automatic[owner]),
            )
        });
    }
    let reserved: HashSet<_> = names[..program.own_count()]
        .iter()
        .map(|name| path_key(name))
        .collect();
    let mut counter = 0;
    for owner in 0..program.own_count() {
        if overrides.classes.contains_key(&owner) {
            continue;
        }
        let conflict = |name: &str, namespace: &Namespace| {
            namespace.types.contains_key(&path_key(name))
                || namespace.packages.contains_key(&path_key(name))
                || namespace.external_types.contains(name)
                || namespace.external_packages.contains(name)
                || roots.contains(simple_name(name))
        };
        if conflict(&names[owner], &namespace) {
            loop {
                let candidate = join(
                    &groups[package(&program.class(owner).name)],
                    &format!("class_{counter}"),
                );
                counter += 1;
                if !conflict(&candidate, &namespace) && !reserved.contains(&path_key(&candidate)) {
                    names[owner] = candidate;
                    break;
                }
            }
        }
        namespace
            .types
            .insert(path_key(&names[owner]), names[owner].clone());
    }
    let check_boundaries =
        (0..program.own_count()).any(|id| package(&names[id]) != package(&automatic[id]));
    Ok(Placement {
        classes: names,
        check_boundaries,
    })
}

fn placement_error(
    message: String,
    names: &[&str],
    overrides: &NameOverrides,
    origins: &MappingOrigins,
) -> MappingDiagnostic {
    let matches = |name: &str| {
        names.iter().any(|wanted| {
            name == *wanted
                || name
                    .strip_prefix(wanted)
                    .is_some_and(|tail| tail.starts_with('/'))
        })
    };
    MappingDiagnostic::with_origins(
        message,
        overrides
            .classes
            .iter()
            .filter(|(_, name)| matches(name))
            .filter_map(|(id, _)| origins.classes.get(id).cloned())
            .chain(
                origins
                    .reserved_classes
                    .iter()
                    .filter(|(name, _)| matches(name))
                    .map(|(_, origin)| origin.clone()),
            ),
    )
    .hint("choose distinct class and package names, including on case-insensitive filesystems")
}

#[derive(Default)]
struct Namespace {
    /// Libraries occupy Java's case-sensitive namespace but create no files.
    external_types: BTreeSet<String>,
    external_packages: BTreeSet<String>,
    /// Owned classes and generated domains share portable output paths.
    types: BTreeMap<String, String>,
    packages: BTreeMap<String, String>,
}

impl Namespace {
    fn package_conflict(&self, name: &str) -> Option<&str> {
        package_prefixes(name).find_map(|prefix| {
            self.types
                .get(&path_key(prefix))
                .or_else(|| self.external_types.get(prefix))
                .or_else(|| {
                    self.packages
                        .get(&path_key(prefix))
                        .filter(|previous| previous.as_str() != prefix)
                })
                .map(String::as_str)
        })
    }

    fn reserve_package(
        &mut self,
        name: &str,
        overrides: &NameOverrides,
        origins: &MappingOrigins,
    ) -> Result<()> {
        if let Some(conflict) = self.package_conflict(name) {
            bail!(placement_error(
                format!("authored or source-only package {name} collides with {conflict}"),
                &[name, conflict],
                overrides,
                origins
            ));
        }
        for prefix in package_prefixes(name) {
            self.packages.insert(path_key(prefix), prefix.to_owned());
        }
        Ok(())
    }
}

/// Only references which are part of declarations or bytecode impose access
/// requirements. Unused constant-pool entries must not prevent a safe move.
pub(super) fn validate_boundaries(
    program: &Program<'_>,
    names: &[String],
    fields: &[Vec<String>],
    origins: &MappingOrigins,
) -> Result<()> {
    let boundaries = Boundaries {
        program,
        names,
        fields,
        origins,
    };
    for owner in 0..program.own_count() {
        let class = program.class(owner);
        for parent in class.super_name.iter().chain(&class.interfaces) {
            boundaries.type_reference(owner, parent)?;
        }
        for field in &class.fields {
            boundaries.descriptor(owner, &field.descriptor)?;
        }
        for method in &class.methods {
            boundaries.descriptor(owner, &method.descriptor)?;
            for exception in &method.exceptions {
                boundaries.type_reference(owner, exception)?;
            }
            let Some(code) = &method.code else { continue };
            let mut protected_receivers = false;
            for handler in &code.exception_table {
                if let Some(exception) = &handler.catch_type {
                    boundaries.type_reference(owner, exception)?;
                }
            }
            for instruction in decode(&code.bytes)? {
                let index = match instruction.operand {
                    Operand::Constant(index)
                    | Operand::InterfaceCall { index, .. }
                    | Operand::MultiArray { index, .. } => index,
                    _ => continue,
                };
                match class.constant_pool.entries.get(index as usize) {
                    Some(Constant::Class(index)) => {
                        let name = class.constant_pool.utf8(*index)?;
                        if name.starts_with('[') {
                            boundaries.descriptor(owner, name)?;
                        } else {
                            boundaries.type_reference(owner, name)?;
                        }
                    }
                    Some(Constant::Fieldref { .. })
                    | Some(Constant::Methodref { .. })
                    | Some(Constant::InterfaceMethodref { .. }) => {
                        let member = class.constant_pool.member_ref(index)?;
                        boundaries.type_reference(owner, member.owner)?;
                        boundaries.descriptor(owner, member.descriptor)?;
                        let field = matches!(instruction.opcode, 0xb2..=0xb5);
                        let target = if field {
                            program.resolve_field(member.owner, member.name, member.descriptor)
                        } else {
                            program.resolve_method(member.owner, member.name, member.descriptor)
                        };
                        if let Some(target) = target {
                            let flags = if field {
                                program.field(target).access_flags
                            } else {
                                program.method(target).access_flags
                            };
                            protected_receivers |=
                                boundaries.member_reference(owner, target, flags, member)?;
                        }
                    }
                    _ => {}
                }
            }
            if protected_receivers {
                boundaries.protected_receivers(owner, method)?;
            }
        }
        // A package-private override can lose dispatch even without a direct
        // call in either class. The original family must remain an override.
        for &ancestor in program.ancestors(owner) {
            if !boundaries.separated(owner, ancestor) {
                continue;
            }
            for method in &program.class(ancestor).methods {
                if method.name.starts_with('<')
                    || method.access_flags & (PUBLIC | PRIVATE | PROTECTED | STATIC) != 0
                {
                    continue;
                }
                if program
                    .declared_method(owner, &method.name, &method.descriptor)
                    .is_some_and(|id| program.method(id).access_flags & (PRIVATE | STATIC) == 0)
                {
                    bail!(boundaries.error(owner, ancestor, format!(
                        "authored placement separates package-private override {}.{}{} from {} ({} and {})",
                        class.name,
                        method.name,
                        method.descriptor,
                        program.class(ancestor).name,
                        names[owner],
                        names[ancestor]
                    )));
                }
            }
        }
    }
    Ok(())
}

struct Boundaries<'a, 'p> {
    program: &'a Program<'p>,
    names: &'a [String],
    fields: &'a [Vec<String>],
    origins: &'a MappingOrigins,
}

impl Boundaries<'_, '_> {
    fn error(&self, from: ClassId, target: ClassId, message: String) -> MappingDiagnostic {
        MappingDiagnostic::with_origins(message, [from, target].into_iter().flat_map(|id| {
            self.origins.classes.iter().filter(move |(candidate, _)| **candidate == id
                || package(&self.program.class(**candidate).name) == package(&self.program.class(id).name)
                    && package(&self.names[**candidate]) == package(&self.names[id]))
                .map(|(_, origin)| origin.clone())
        })).hint("keep classes that depend on package access together, or correct the requested package move")
    }
    fn separated(&self, from: ClassId, target: ClassId) -> bool {
        package(&self.program.class(from).name) == package(&self.program.class(target).name)
            && package(&self.names[from]) != package(&self.names[target])
    }

    fn type_reference(&self, from: ClassId, name: &str) -> Result<()> {
        let Some(target) = self.program.id(name) else {
            return Ok(());
        };
        ensure!(
            package(&self.names[from]).is_empty() || !package(&self.names[target]).is_empty(),
            self.error(
                from,
                target,
                format!(
                    "authored placement makes {} ({}) reference {} ({}) in the unnamed package",
                    self.program.class(from).name,
                    self.names[from],
                    name,
                    self.names[target]
                )
            )
        );
        ensure!(
            !self.separated(from, target) || self.program.class(target).access_flags & PUBLIC != 0,
            self.error(from, target, format!("authored placement removes access from {} ({}) to package-private class {} ({})",
            self.program.class(from).name,
            self.names[from],
            name,
            self.names[target]))
        );
        Ok(())
    }

    fn descriptor(&self, from: ClassId, descriptor: &str) -> Result<()> {
        fn visit(ty: &JvmType, boundaries: &Boundaries<'_, '_>, from: ClassId) -> Result<()> {
            match ty {
                JvmType::Object(name) => boundaries.type_reference(from, name),
                JvmType::Array(element) => visit(element, boundaries, from),
                _ => Ok(()),
            }
        }
        if descriptor.starts_with('(') {
            let method = MethodDescriptor::parse(descriptor)?;
            for ty in method
                .parameters
                .iter()
                .chain(std::iter::once(&method.return_type))
            {
                visit(ty, self, from)?;
            }
        } else {
            visit(&JvmType::parse(descriptor)?, self, from)?;
        }
        Ok(())
    }

    fn member_reference(
        &self,
        from: ClassId,
        target: MemberId,
        flags: u16,
        member: crate::classfile::MemberRef<'_>,
    ) -> Result<bool> {
        if !self.separated(from, target.class) || flags & (PUBLIC | PRIVATE) != 0 {
            return Ok(false);
        }
        ensure!(
            flags & PROTECTED != 0 && self.program.ancestors(from).contains(&target.class),
            self.error(
                from,
                target.class,
                format!(
                    "authored placement removes package access from {} ({}) to {}.{}{} ({})",
                    self.program.class(from).name,
                    self.names[from],
                    self.program.class(target.class).name,
                    member.name,
                    member.descriptor,
                    self.names[target.class]
                )
            )
        );
        Ok(flags & STATIC == 0)
    }

    fn protected_receivers(&self, from: ClassId, method: &crate::classfile::Method) -> Result<()> {
        use crate::ir::{Member, MethodContext, Statement, UpdateTarget, ValueKind};
        let assignable = |from: &JvmType, to: &JvmType| self.program.assignable(from, to);
        let ir = crate::ir::decompile_method_with_context(
            self.program.class(from),
            method,
            &MethodContext {
                reference_assignable: Some(&assignable),
                ..MethodContext::default()
            },
        )?;
        let check = |member: &Member, field: bool, receiver: Option<crate::ir::ValueId>| {
            let target = if field {
                self.program
                    .resolve_field(&member.owner, &member.name, &member.descriptor)
            } else {
                self.program
                    .resolve_method(&member.owner, &member.name, &member.descriptor)
            };
            let Some(target) = target else { return Ok(()) };
            let flags = if field {
                self.program.field(target).access_flags
            } else {
                self.program.method(target).access_flags
            };
            if !self.separated(from, target.class) || flags & PROTECTED == 0 || flags & STATIC != 0
            {
                return Ok(());
            }
            let accessible_receiver = receiver.is_some_and(|receiver| {
                let this = proven_this(&ir, receiver, &mut BTreeSet::new());
                if member.name == "<init>" {
                    return this;
                }
                let value = &ir.values[receiver];
                let ty = value.local.map_or(&value.ty, |local| &ir.locals[local].ty);
                let JvmType::Object(name) = ty else {
                    return false;
                };
                if !self
                    .program
                    .is_subtype(name, &self.program.class(from).name)
                {
                    return false;
                }
                if !field {
                    return true;
                }
                let Some(actual) = self.program.id(name) else {
                    return false;
                };
                let name = &self.fields[target.class][target.index];
                if source_field_resolves(self.program, self.fields, actual, name, target) {
                    return true;
                }
                // Emission can select an inherited hidden field with `super`
                // only on this receiver. Other receivers need an explicit cast.
                let super_class = self
                    .program
                    .class(from)
                    .super_name
                    .as_deref()
                    .and_then(|name| self.program.id(name));
                if this
                    && super_class.is_some_and(|owner| {
                        source_field_resolves(self.program, self.fields, owner, name, target)
                    })
                {
                    return true;
                }
                let cast_owner = self
                    .program
                    .id(&member.owner)
                    .filter(|&owner| {
                        source_field_resolves(self.program, self.fields, owner, name, target)
                    })
                    .unwrap_or(target.class);
                self.program.is_subtype(
                    &self.program.class(cast_owner).name,
                    &self.program.class(from).name,
                )
            });
            ensure!(
                accessible_receiver,
                self.error(from, target.class, format!("authored placement removes protected receiver access in {}.{}{} to {}.{}{}",
                self.program.class(from).name,
                method.name,
                method.descriptor,
                self.program.class(target.class).name,
                member.name,
                member.descriptor))
            );
            Ok(())
        };
        for value in &ir.values {
            match &value.kind {
                ValueKind::Field { field, receiver }
                | ValueKind::Update {
                    target: UpdateTarget::Field { field, receiver },
                    ..
                }
                | ValueKind::CompoundUpdate {
                    target: UpdateTarget::Field { field, receiver },
                    ..
                } => {
                    check(field, true, *receiver)?;
                }
                ValueKind::Call {
                    method, receiver, ..
                } => check(method, false, *receiver)?,
                ValueKind::New {
                    class, descriptor, ..
                } => check(
                    &Member {
                        owner: class.clone(),
                        name: "<init>".into(),
                        descriptor: descriptor.clone(),
                    },
                    false,
                    None,
                )?,
                _ => {}
            }
        }
        for block in &ir.blocks {
            for statement in &block.statements {
                match statement {
                    Statement::PutField {
                        field, receiver, ..
                    } => check(field, true, *receiver)?,
                    Statement::ConstructorCall {
                        method, receiver, ..
                    } => check(method, false, Some(*receiver))?,
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

fn proven_this(
    ir: &crate::ir::MethodIr,
    id: crate::ir::ValueId,
    visiting: &mut BTreeSet<crate::ir::ValueId>,
) -> bool {
    use crate::ir::ValueKind;
    if !visiting.insert(id) {
        return true;
    }
    match &ir.values[id].kind {
        ValueKind::Alias(value) => proven_this(ir, *value, visiting),
        ValueKind::Parameter { index: None, .. } => true,
        ValueKind::Phi { inputs, .. } => {
            !inputs.is_empty()
                && inputs
                    .iter()
                    .all(|input| proven_this(ir, input.value, visiting))
        }
        _ => false,
    }
}

/// Source field selection shared with rendering: declarations hide ancestors,
/// and an inherited interface field can make a superclass field ambiguous.
pub(super) fn source_field_resolves(
    program: &Program<'_>,
    fields: &[Vec<String>],
    owner: ClassId,
    name: &str,
    target: MemberId,
) -> bool {
    let mut current = Some(owner);
    while let Some(class) = current {
        if let Some(index) = fields[class].iter().position(|field| field == name) {
            return target == MemberId { class, index };
        }
        if program
            .class(class)
            .interfaces
            .iter()
            .filter_map(|name| program.id(name))
            .flat_map(|interface| {
                std::iter::once(interface).chain(program.ancestors(interface).iter().copied())
            })
            .any(|interface| fields[interface].iter().any(|field| field == name))
        {
            return false;
        }
        current = program
            .class(class)
            .super_name
            .as_deref()
            .and_then(|name| program.id(name));
    }
    false
}
