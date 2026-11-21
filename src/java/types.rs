use crate::classfile::{
    Constant, FINAL, INTERFACE, JvmType, PRIVATE, PROTECTED, PUBLIC, STATIC, package,
};
use crate::hash::{HashMap, HashSet};
use crate::{
    naming::NamePlan,
    program::{ClassId, MemberId, Program},
};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use super::syntax::Expr;

/// Import decisions belong to a whole source unit, never to expression order.
pub struct TypeNames<'a> {
    pub names: &'a NamePlan,
    pub owner: ClassId,
    pub(super) units: &'a super::SourceUnits,
    spellings: Rc<Spellings>,
    shadowing_types: [&'a HashSet<String>; 2],
    used_imports: RefCell<BTreeSet<String>>,
}

#[derive(Clone)]
struct Spellings {
    by_original: HashMap<String, TypeSpelling>,
    shadowing_values: HashSet<String>,
    local_reservations: HashSet<String>,
}

#[derive(Clone, Default)]
struct TypeSpelling {
    canonical: String,
    short: Option<String>,
    needs_import: bool,
    // A field on an enclosing class can capture a later component of a
    // qualified name in expression context (Outer.Inner.staticField).
    qualified_value_shadowed: bool,
}

impl TypeSpelling {
    fn member(
        member: &super::units::Member,
        id: ClassId,
        owner_package: &str,
        same_package: bool,
        visible: &HashMap<String, Option<ClassId>>,
    ) -> Self {
        let path = member.path(same_package);
        let short = if visible.get(&member.simple) == Some(&Some(id)) {
            Some(member.simple.as_str())
        } else if same_package {
            let relative = if owner_package.is_empty() {
                &path.name
            } else {
                &path.name[owner_package.len() + 1..]
            };
            (!visible.contains_key(head(relative))).then_some(relative)
        } else {
            None
        };
        Self {
            canonical: path.name.clone(),
            short: short.map(str::to_owned),
            needs_import: false,
            qualified_value_shadowed: path.field_shadowed,
        }
    }

    /// Importing another type at either head changes how Java binds the path.
    fn qualifier_roots(&self) -> impl Iterator<Item = &str> {
        [Some(self.canonical.as_str()), self.short.as_deref()]
            .into_iter()
            .flatten()
            .filter_map(|name| name.split_once('.').map(|(root, _)| root))
    }

    fn short_qualifier(&self, shadowing: &HashSet<String>) -> Option<&str> {
        self.short.as_deref().filter(|name| {
            !shadowing.contains(head(name))
                && (!name.contains('.') || !self.qualified_value_shadowed)
        })
    }
}

fn head(name: &str) -> &str {
    name.split(['.', '/']).next().unwrap()
}

impl<'a> TypeNames<'a> {
    pub fn with_methods(
        program: &Program<'_>,
        names: &'a NamePlan,
        units: &'a super::SourceUnits,
        owner: ClassId,
        methods: impl IntoIterator<Item = MemberId>,
        generated_types: &[JvmType],
        mappings: Option<&crate::mappings::MappingPlan>,
    ) -> Self {
        let class = program.class(owner);
        let mut shadowing_values = HashSet::default();
        let mut candidates = source_candidates(program, owner, generated_types, mappings);
        for member in methods {
            collect_descriptor(&program.method(member).descriptor, &mut candidates);
            candidates.extend(program.method_throws(member).iter().cloned());
            shadowing_values.extend(names.parameter_names(member).into_iter().flatten().cloned());
        }
        let visible = visible_member_types(program, names, units, owner, &mut shadowing_values);
        let types = plan_imports(program, names, units, owner, candidates, &visible);
        // Local variables can shadow expression qualifiers (System, java,
        // Outer in Outer.Inner). Final static stores can also be unqualified.
        let mut local_reservations: HashSet<_> = class
            .fields
            .iter()
            .enumerate()
            .filter(|(_, field)| field.access_flags & (STATIC | FINAL) == STATIC | FINAL)
            .map(|(index, _)| {
                names
                    .field_name(MemberId {
                        class: owner,
                        index,
                    })
                    .to_owned()
            })
            .collect();
        for spelling in types.values() {
            local_reservations.insert(head(&spelling.canonical).to_owned());
            if let Some(short) = spelling.short_qualifier(&shadowing_values) {
                local_reservations.insert(head(short).to_owned());
            }
        }
        local_reservations.insert("java".into());
        Self {
            names,
            units,
            owner,
            spellings: Rc::new(Spellings {
                by_original: types,
                shadowing_values,
                local_reservations,
            }),
            shadowing_types: names.visible_package_types(owner),
            used_imports: RefCell::default(),
        }
    }

    /// A speculative body shares its type bindings and records only imports it requests.
    pub(super) fn for_artifact(&self) -> Self {
        Self {
            names: self.names,
            units: self.units,
            owner: self.owner,
            spellings: Rc::clone(&self.spellings),
            shadowing_types: self.shadowing_types,
            used_imports: RefCell::default(),
        }
    }

    pub(super) fn simple_name(&self) -> &str {
        self.units.simple(self.names, self.owner)
    }

    /// A class name in a type context, where fields and locals cannot shadow it.
    pub fn class(&self, original: &str) -> String {
        if let Some(spelling) = self.spellings.by_original.get(original) {
            return match &spelling.short {
                Some(short) => self.use_short(spelling, short),
                None => spelling.canonical.clone(),
            };
        }
        self.full_name(original).replace('/', ".")
    }

    /// A static member receiver. An authored value can shadow both the short
    /// class name and its package root, and unnamed-package classes have no
    /// qualified alternative. A typed null remains unambiguous in a type
    /// context and has no evaluation or null-check effects for static access.
    pub(super) fn qualifier(&self, original: &str) -> Expr {
        match self.named_qualifier(original) {
            Some(name) => Expr::atom(name),
            None => Expr::cast(self.class(original), Expr::null()).grouped(),
        }
    }

    /// Constant expressions require a name, not a static access through a
    /// primary expression. Semantic presentation must retain the literal when
    /// authored bindings obscure every available class qualifier.
    pub(super) fn named_qualifier(&self, original: &str) -> Option<String> {
        if let Some((spelling, short)) = self.short_qualifier(original) {
            return Some(self.use_short(spelling, short));
        }
        self.full_qualifier(original)
            .map(|name| name.replace('/', "."))
    }

    /// Probe semantic references without requesting an unused import.
    pub(super) fn can_name_qualifier(&self, original: &str) -> bool {
        self.short_qualifier(original).is_some() || self.full_qualifier(original).is_some()
    }

    fn short_qualifier(&self, original: &str) -> Option<(&TypeSpelling, &str)> {
        let spelling = self.spellings.by_original.get(original)?;
        Some((
            spelling,
            spelling.short_qualifier(&self.spellings.shadowing_values)?,
        ))
    }

    fn full_qualifier<'s>(&'s self, original: &'s str) -> Option<&'s str> {
        let (name, field_shadowed) = self.full_path(original);
        let root = head(name);
        (!field_shadowed
            && !self.spellings.shadowing_values.contains(root)
            && !self
                .shadowing_types
                .iter()
                .any(|types| types.contains(root)))
        .then_some(name)
    }

    fn use_short(&self, spelling: &TypeSpelling, short: &str) -> String {
        if spelling.needs_import {
            let mut imports = self.used_imports.borrow_mut();
            if !imports.contains(&spelling.canonical) {
                imports.insert(spelling.canonical.clone());
            }
        }
        short.to_owned()
    }

    fn full_name<'s>(&'s self, original: &'s str) -> &'s str {
        self.full_path(original).0
    }

    fn full_path<'s>(&'s self, original: &'s str) -> (&'s str, bool) {
        if let Some(spelling) = self.spellings.by_original.get(original) {
            return (&spelling.canonical, spelling.qualified_value_shadowed);
        }
        if let Some(id) = self.names.class_id(original)
            && let Some(member) = self.units.member(id)
        {
            let path = member.path(
                package(self.names.class_name(self.owner)) == package(self.names.class_name(id)),
            );
            return (&path.name, path.field_shadowed);
        }
        (self.names.class_name_for(original), false)
    }

    /// Finalized after the complete source unit has bound its type references.
    pub(super) fn take_imports(&self) -> BTreeSet<String> {
        self.used_imports.take()
    }

    pub(super) fn commit_imports(&self, imports: BTreeSet<String>) {
        self.used_imports.borrow_mut().extend(imports);
    }

    pub fn ty(&self, ty: &JvmType) -> String {
        match ty {
            JvmType::Object(name) => self.class(name),
            JvmType::Array(element) => format!("{}[]", self.ty(element)),
            _ => self.names.type_name(ty),
        }
    }

    /// A generated nested class can shadow imported/simple types and the first
    /// component of qualified type names already selected for this source unit.
    pub(super) fn reserves_type_name(&self, candidate: &str) -> bool {
        self.spellings.by_original.values().any(|spelling| {
            spelling
                .short
                .as_deref()
                .is_some_and(|short| head(short) == candidate)
                || head(&spelling.canonical) == candidate
        })
    }

    pub(super) fn local_reservations(&self) -> &HashSet<String> {
        &self.spellings.local_reservations
    }
}

/// Independent declarations may have selected two different imports, or a
/// sibling may depend on an implicit java.lang or same-package binding.
pub(super) fn reconcile_unit(classes: &mut [super::Declaration<'_>]) {
    let mut seen = HashMap::default();
    let mut conflicts = HashSet::default();
    for class in classes.iter() {
        let types = &class.types;
        for (original, spelling) in &types.spellings.by_original {
            if let Some(short) = &spelling.short
                && !short.contains('.')
                && seen
                    .insert(short, original)
                    .is_some_and(|old| old != original)
            {
                conflicts.insert(short.clone());
            }
            conflicts.extend(spelling.qualifier_roots().map(str::to_owned));
        }
        conflicts.insert(types.simple_name().to_owned());
    }
    // A hoisted helper can shadow an import or qualifier used only by a sibling.
    let helper_collisions: HashSet<_> = seen
        .keys()
        .map(|name| name.as_str())
        .chain(conflicts.iter().map(String::as_str))
        .filter(|name| name.starts_with(super::enclosing::SUPPORT_PREFIX))
        .map(str::to_owned)
        .collect();
    for class in classes {
        let captured = class.types.units.capture(class.types.owner).is_some();
        let bindings = Rc::make_mut(&mut class.types.spellings);
        if captured {
            bindings
                .local_reservations
                .extend(helper_collisions.iter().cloned());
        }
        // Locals referring to same-package types can keep their short names
        // once all imports for the colliding name have been suppressed.
        for spelling in bindings.by_original.values_mut() {
            if spelling.needs_import
                && spelling
                    .short
                    .as_ref()
                    .is_some_and(|short| conflicts.contains(short))
            {
                spelling.short = None;
            }
        }
    }
}

fn source_candidates(
    program: &Program<'_>,
    owner: ClassId,
    generated_types: &[JvmType],
    mappings: Option<&crate::mappings::MappingPlan>,
) -> BTreeSet<String> {
    let class = program.class(owner);
    let mut candidates = declared_types(program, owner);
    for name in class.constant_pool.class_constants() {
        collect_class(name, &mut candidates);
    }
    for descriptor in class.constant_pool.member_descriptors() {
        collect_descriptor(descriptor, &mut candidates);
    }

    // Generated and semantic references can introduce types absent from the class file.
    // Include every possible owner in the same ambiguity/reservation plan.
    // All imports, including constant-pool candidates, are requested only
    // when source binding actually selects the type's short spelling.
    candidates.extend(
        super::literals::STANDARD_OWNERS
            .iter()
            .map(|name| (*name).to_owned()),
    );
    candidates.extend(
        generated_types
            .iter()
            .filter_map(JvmType::object_name)
            .map(str::to_owned),
    );
    if let Some(mappings) = mappings {
        for domain in mappings.domains() {
            if domain.generated() {
                candidates.insert(domain.owner.clone());
            }
            for constant in &domain.values {
                if let Some(field) = constant.source {
                    let owner = &program.class(field.class).name;
                    candidates.insert(owner.clone());
                }
            }
        }
    }
    candidates
}

fn visible_member_types(
    program: &Program<'_>,
    names: &NamePlan,
    units: &super::SourceUnits,
    owner: ClassId,
    shadowing_values: &mut HashSet<String>,
) -> HashMap<String, Option<ClassId>> {
    let class = program.class(owner);
    let owner_package = package(names.class_name(owner));
    for index in 0..class.methods.len() {
        shadowing_values.extend(
            names
                .parameter_names(MemberId {
                    class: owner,
                    index,
                })
                .into_iter()
                .flatten()
                .cloned(),
        );
    }
    let mut member_types = HashMap::default();
    for id in units.scopes(program, owner) {
        let same_package = package(names.class_name(id)) == owner_package;
        units.visible_members(program, id, owner, same_package, &mut member_types);
        let declaration = program.class(id);
        for index in 0..declaration.fields.len() {
            shadowing_values.insert(names.field_name(MemberId { class: id, index }).to_owned());
        }
        if !program.is_own(id) {
            for member in &declaration.inner_classes {
                if member.outer.as_deref() == Some(&declaration.name)
                    && (member.access_flags & (PUBLIC | PROTECTED) != 0
                        || member.access_flags & PRIVATE == 0 && same_package)
                    && let Some(name) = &member.inner_name
                {
                    shadowing_values.insert(name.clone());
                    member_types.insert(name.clone(), None);
                }
            }
        }
    }
    member_types
}

fn plan_imports(
    program: &Program<'_>,
    names: &NamePlan,
    units: &super::SourceUnits,
    owner: ClassId,
    candidates: BTreeSet<String>,
    member_types: &HashMap<String, Option<ClassId>>,
) -> HashMap<String, TypeSpelling> {
    let owner_package = package(names.class_name(owner));
    let mut by_simple = HashMap::<String, Vec<String>>::default();
    let mut types = HashMap::default();
    let mut package_roots = HashSet::default();
    for original in candidates {
        let binary = names.class_name_for(&original);
        if let Some(id) = names.class_id(&original)
            && let Some(member) = units.member(id)
        {
            let same_package = package(binary) == owner_package;
            let spelling =
                TypeSpelling::member(member, id, owner_package, same_package, member_types);
            package_roots.extend(spelling.qualifier_roots().map(str::to_owned));
            if !same_package
                && spelling.short.is_none()
                && member.exported_as.is_none()
                && units.importable(program, id)
            {
                by_simple
                    .entry(member.simple.clone())
                    .or_default()
                    .push(original.clone());
            }
            types.insert(original, spelling);
            continue;
        }
        let emitted = source_name(program, names, units, &original, &mut HashSet::default());
        if let Some((root, _)) = emitted.split_once('.') {
            package_roots.insert(root.to_owned());
        }
        let short = emitted.rsplit('.').next().unwrap().to_owned();
        types.insert(
            original.clone(),
            TypeSpelling {
                canonical: emitted,
                ..Default::default()
            },
        );
        by_simple.entry(short).or_default().push(original);
    }
    let shadowing_types = names.visible_package_types(owner);
    for (short, originals) in by_simple {
        if member_types.contains_key(&short) {
            continue;
        }
        // A same-package type already has an unambiguous simple spelling.
        // Prefer it over an import with the same name: qualifying it can
        // itself be illegal when the package root names a visible type
        // (for example Class.Player and java.lang.Class).
        let original = originals
            .iter()
            .find(|original| package(names.class_name_for(original)) == owner_package)
            .or_else(|| {
                originals.iter().find(|original| {
                    types[*original]
                        .canonical
                        .split_once('.')
                        .is_some_and(|(root, _)| {
                            shadowing_types.iter().any(|types| types.contains(root))
                        })
                })
            })
            .or_else(|| (originals.len() == 1).then(|| &originals[0]));
        let Some(original) = original else { continue };
        let emitted = names.class_name_for(original);
        let type_package = package(emitted);
        // Outer.Inner reserves Outer against imports. The same-package
        // top-level Outer is exactly the binding that spelling needs.
        if type_package != owner_package && package_roots.contains(&short) {
            continue;
        }
        // A same-package class can hide an implicit java.lang import even
        // when no original constant-pool entry names that class.
        if type_package == "java/lang"
            && type_package != owner_package
            && shadowing_types[0].contains(&short)
        {
            continue;
        }
        // Constant pools can retain unused inaccessible declaring types.
        // An import itself is a checked source reference, even when no
        // emitted expression names that type.
        if type_package != owner_package
            && program
                .id(original)
                .is_some_and(|id| program.class(id).access_flags & PUBLIC == 0)
        {
            continue;
        }
        let spelling = types.get_mut(original).unwrap();
        let is_nested = spelling.canonical != emitted.replace('/', ".");
        spelling.needs_import = is_nested
            || (type_package != owner_package
                && type_package != "java/lang"
                && !type_package.is_empty());
        spelling.short = Some(short);
    }
    types
}

fn declared_types(program: &Program<'_>, owner: ClassId) -> BTreeSet<String> {
    let class = program.class(owner);
    let mut candidates = BTreeSet::from([class.name.clone()]);
    candidates.extend(class.super_name.iter().cloned());
    candidates.extend(class.interfaces.iter().cloned());
    for field in &class.fields {
        collect_descriptor(&field.descriptor, &mut candidates);
    }
    for (index, method) in class.methods.iter().enumerate() {
        collect_descriptor(&method.descriptor, &mut candidates);
        candidates.extend(method.exceptions.iter().cloned());
        // A propagated exception can appear in a throws clause or a generated
        // catch even when the caller's constant pool never names that type.
        let member = MemberId {
            class: owner,
            index,
        };
        let throws = program.throws_plan();
        candidates.extend(
            throws
                .declared(member)
                .iter()
                .chain(throws.boundary(member))
                .cloned(),
        );
    }
    candidates
}

/// Only actual declarations and code references constrain member placement.
/// In particular, InnerClasses records alone do not make a class a user.
pub(super) fn used_member_types(program: &Program<'_>, owner: ClassId) -> BTreeSet<String> {
    use crate::bytecode::decode;
    let class = program.class(owner);
    let mut used = declared_types(program, owner);
    let pool = &class.constant_pool;
    // Only proven owner$member identities participate in source placement.
    // Most classes in stripped J2ME archives cannot name one in an instruction.
    let may_reference_member = pool
        .class_constants()
        .chain(pool.member_descriptors())
        .any(|name| name.contains('$'));
    let mut seen = HashSet::default();
    for code in class
        .methods
        .iter()
        .filter_map(|method| method.code.as_ref())
    {
        used.extend(
            code.exception_table
                .iter()
                .filter_map(|handler| handler.catch_type.clone()),
        );
        for local in &code.local_variables {
            collect_descriptor(&local.descriptor, &mut used);
        }
        if !may_reference_member {
            continue;
        }
        let Ok(instructions) = decode(&code.bytes) else {
            continue;
        };
        for index in instructions
            .iter()
            .filter_map(|op| op.operand.constant_index())
        {
            if !seen.insert(index) {
                continue;
            }
            match pool.entries.get(usize::from(index)) {
                Some(Constant::Class(name)) => {
                    if let Ok(name) = pool.utf8(*name) {
                        collect_class(name, &mut used);
                    }
                }
                Some(
                    Constant::Fieldref { .. }
                    | Constant::Methodref { .. }
                    | Constant::InterfaceMethodref { .. },
                ) => {
                    if let Ok(reference) = pool.member_ref(index) {
                        collect_class(reference.owner, &mut used);
                        collect_descriptor(reference.descriptor, &mut used);
                    }
                }
                _ => {}
            }
        }
    }
    used.retain(|name| name.contains('$'));
    used
}

fn collect_class(name: &str, types: &mut BTreeSet<String>) {
    if name.starts_with('[') {
        collect_descriptor(name, types);
    } else {
        types.insert(name.to_owned());
    }
}

fn collect_descriptor(descriptor: &str, names: &mut BTreeSet<String>) {
    if let Ok(types) = crate::classfile::descriptor::types(descriptor) {
        names.extend(types.filter_map(|ty| ty.object_name().map(str::to_owned)));
    }
}

/// Source nesting and JVM binary names differ for both program and library types.
fn source_name(
    program: &Program<'_>,
    names: &NamePlan,
    units: &super::SourceUnits,
    original: &str,
    visiting: &mut HashSet<String>,
) -> String {
    let flat = names.class_name_for(original).replace('/', ".");
    let Some(id) = program.id(original) else {
        return flat;
    };
    if let Some(member) = units.member(id) {
        return member.canonical.name.clone();
    }
    if program.is_own(id) || !visiting.insert(original.to_owned()) {
        return flat;
    }
    let result = program
        .class(id)
        .inner_classes
        .iter()
        .find(|entry| entry.inner == original)
        .and_then(|entry| entry.outer.as_ref().zip(entry.inner_name.as_ref()))
        .map(|(outer, simple)| {
            format!(
                "{}.{simple}",
                source_name(program, names, units, outer, visiting)
            )
        })
        .unwrap_or(flat);
    visiting.remove(original);
    result
}

/// JVM reference tests accept unrelated types. A direct Java conversion needs
/// a known legal relation; otherwise an Object view preserves the JVM operation.
pub fn cast_compatible(
    program: &Program<'_>,
    names: &NamePlan,
    from: &JvmType,
    to: &JvmType,
) -> bool {
    if from == to || program.assignable(from, to) || program.assignable(to, from) {
        return true;
    }
    match (from, to) {
        (JvmType::Array(left), JvmType::Array(right)) => {
            left.is_reference()
                && right.is_reference()
                && cast_compatible(program, names, left, right)
        }
        (JvmType::Array(_), JvmType::Object(_)) | (JvmType::Object(_), JvmType::Array(_)) => false,
        (JvmType::Object(left), JvmType::Object(right)) => {
            let (Some(left), Some(right)) = (program.id(left), program.id(right)) else {
                // Export-only placeholders do not establish runtime hierarchy.
                // They can be unrelated classes in the compilation classpath,
                // so missing metadata cannot prove a direct cast legal.
                return false;
            };
            let left_flags = program.class(left).access_flags;
            let right_flags = program.class(right).access_flags;
            match (left_flags & INTERFACE != 0, right_flags & INTERFACE != 0) {
                (false, false) => false,
                (true, true) => interfaces_compatible(program, names, left, right),
                (true, false) => right_flags & FINAL == 0,
                (false, true) => left_flags & FINAL == 0,
            }
        }
        _ => true,
    }
}

fn interfaces_compatible(
    program: &Program<'_>,
    names: &NamePlan,
    left: ClassId,
    right: ClassId,
) -> bool {
    // JLS second edition 5.5: interfaces with equal method signatures but
    // different returns cannot be cast directly, even for covariant returns.
    // Use emitted names because name repair can remove or introduce a clash.
    let methods = |owner| {
        std::iter::once(owner)
            .chain(program.ancestors(owner).iter().copied())
            .filter(|&id| program.class(id).access_flags & INTERFACE != 0)
            .flat_map(|class| {
                program.class(class).methods.iter().enumerate().filter_map(
                    move |(index, method)| {
                        if method.name.starts_with('<')
                            || method.access_flags & (PRIVATE | STATIC) != 0
                        {
                            return None;
                        }
                        let (parameters, returns) = method.descriptor.split_once(')')?;
                        Some((
                            (names.method_name(MemberId { class, index }), parameters),
                            returns,
                        ))
                    },
                )
            })
    };
    let left: HashMap<_, _> = methods(left).collect();
    methods(right)
        .all(|(signature, returns)| left.get(&signature).is_none_or(|other| *other == returns))
}
