use crate::classfile::{Constant, JvmType, MethodDescriptor};
use crate::{
    naming::NamePlan,
    program::{ClassId, MemberId, Program},
};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

use super::syntax::Expr;

/// Import decisions belong to a whole source unit, never to expression order.
#[derive(Clone)]
pub struct TypeNames<'a> {
    pub names: &'a NamePlan,
    pub owner: ClassId,
    simple_types: Rc<HashMap<String, String>>,
    simple_qualifiers: Rc<HashMap<String, String>>,
    canonical: Rc<HashMap<String, String>>,
    imports: Rc<HashMap<String, String>>,
    used_imports: RefCell<BTreeSet<String>>,
    shadowing_values: Rc<HashSet<String>>,
    shadowing_types: [&'a HashSet<String>; 2],
    local_reservations: Rc<HashSet<String>>,
}

impl<'a> TypeNames<'a> {
    pub fn with_methods(
        program: &Program<'_>,
        names: &'a NamePlan,
        owner: ClassId,
        methods: impl IntoIterator<Item = MemberId>,
        generated_types: &[JvmType],
        mappings: Option<&crate::mappings::MappingPlan>,
    ) -> Self {
        let owner_name = names.class_name(owner);
        let package = owner_name.rsplit_once('/').map_or("", |(p, _)| p);
        let mut candidates = BTreeSet::new();
        let mut authored_parameters = HashSet::new();
        let class = program.class(owner);
        candidates.insert(class.name.clone());
        candidates.extend(class.super_name.iter().cloned());
        candidates.extend(class.interfaces.iter().cloned());
        for member in methods {
            collect_descriptor(&program.method(member).descriptor, &mut candidates);
            candidates.extend(program.method_throws(member).iter().cloned());
            authored_parameters
                .extend(names.parameter_names(member).into_iter().flatten().cloned());
        }
        for constant in &class.constant_pool.entries {
            if let crate::classfile::Constant::Class(index) = constant
                && let Ok(name) = class.constant_pool.utf8(*index)
            {
                if name.starts_with('[') {
                    if let Ok(ty) = JvmType::parse(name) {
                        collect(&ty, &mut candidates);
                    }
                } else {
                    candidates.insert(name.to_owned());
                }
            }
            if let Constant::NameAndType {
                descriptor_index, ..
            } = constant
                && let Ok(descriptor) = class.constant_pool.utf8(*descriptor_index)
            {
                collect_descriptor(descriptor, &mut candidates);
            }
        }
        for field in &class.fields {
            if let Ok(ty) = JvmType::parse(&field.descriptor) {
                collect(&ty, &mut candidates);
            }
        }
        for (index, method) in class.methods.iter().enumerate() {
            authored_parameters.extend(
                names
                    .parameter_names(MemberId {
                        class: owner,
                        index,
                    })
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
            if let Ok(desc) = crate::classfile::MethodDescriptor::parse(&method.descriptor) {
                for ty in desc
                    .parameters
                    .iter()
                    .chain(std::iter::once(&desc.return_type))
                {
                    collect(ty, &mut candidates);
                }
            }
            candidates.extend(method.exceptions.iter().cloned());
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
        for ty in generated_types {
            collect(ty, &mut candidates);
        }
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
        // Fields, authored parameters and inherited library member types can
        // hide expression qualifiers. Own member types are flattened. Plan
        // against the whole source unit so helpers and bodies agree.
        let mut shadowing_values = authored_parameters;
        for id in std::iter::once(owner).chain(program.ancestors(owner).iter().copied()) {
            let declaration = program.class(id);
            for index in 0..declaration.fields.len() {
                shadowing_values.insert(names.field_name(MemberId { class: id, index }).to_owned());
            }
            if !program.is_own(id) {
                let same_package = names
                    .class_name(id)
                    .rsplit_once('/')
                    .map_or("", |(package, _)| package)
                    == package;
                for member in &declaration.inner_classes {
                    if member.outer.as_deref() == Some(&declaration.name)
                        && (member.access_flags & 0x0005 != 0
                            || member.access_flags & 0x0002 == 0 && same_package)
                        && let Some(name) = &member.inner_name
                    {
                        shadowing_values.insert(name.clone());
                    }
                }
            }
        }
        let mut by_simple = HashMap::<String, Vec<String>>::new();
        let mut canonical = HashMap::new();
        for original in candidates {
            let emitted = source_name(program, names, &original, &mut HashSet::new());
            let short = emitted.rsplit('.').next().unwrap().to_owned();
            canonical.insert(original.clone(), emitted);
            by_simple.entry(short).or_default().push(original);
        }
        let mut simple_types = HashMap::new();
        let mut simple_qualifiers = HashMap::new();
        let mut imports = HashMap::new();
        // Package roots are also obscured by types visible without an import,
        // including java.lang types never mentioned in this constant pool.
        let shadowing_types = names.visible_package_types(owner);
        let package_roots: HashSet<_> = canonical
            .values()
            .filter_map(|name| name.split_once('.').map(|(root, _)| root.to_owned()))
            .collect();
        for (short, originals) in by_simple {
            if package_roots.contains(&short) {
                continue;
            }
            // A same-package type already has an unambiguous simple spelling.
            // Prefer it over an import with the same name: qualifying it can
            // itself be illegal when the package root names a visible type
            // (for example Class.Player and java.lang.Class).
            let original = originals
                .iter()
                .find(|original| {
                    names
                        .class_name_for(original)
                        .rsplit_once('/')
                        .map_or("", |(p, _)| p)
                        == package
                })
                .or_else(|| {
                    originals.iter().find(|original| {
                        canonical[*original]
                            .split_once('.')
                            .is_some_and(|(root, _)| {
                                shadowing_types.iter().any(|types| types.contains(root))
                            })
                    })
                })
                .or_else(|| (originals.len() == 1).then(|| &originals[0]));
            let Some(original) = original else { continue };
            let emitted = names.class_name_for(original);
            let type_package = emitted.rsplit_once('/').map_or("", |(p, _)| p);
            // A same-package class can hide an implicit java.lang import even
            // when no original constant-pool entry names that class.
            if type_package == "java/lang"
                && type_package != package
                && shadowing_types[0].contains(&short)
            {
                continue;
            }
            // Constant pools can retain unused inaccessible declaring types.
            // An import itself is a checked source reference, even when no
            // emitted expression names that type.
            if type_package != package
                && program
                    .id(original)
                    .is_some_and(|id| program.class(id).access_flags & 0x0001 == 0)
            {
                continue;
            }
            let is_nested_library = canonical[original] != emitted.replace('/', ".");
            if is_nested_library
                || (type_package != package
                    && type_package != "java/lang"
                    && !type_package.is_empty())
            {
                imports.insert(original.clone(), canonical[original].clone());
            }
            if !shadowing_values.contains(&short) {
                simple_qualifiers.insert(original.clone(), short.clone());
            }
            simple_types.insert(original.clone(), short);
        }
        // Local variables can shadow expression qualifiers (System, java,
        // Outer in Outer.Inner). Final static stores can also be unqualified.
        let mut local_reservations: HashSet<_> = class
            .fields
            .iter()
            .enumerate()
            .filter(|(_, field)| field.access_flags & 0x0018 == 0x0018)
            .map(|(index, _)| {
                names
                    .field_name(MemberId {
                        class: owner,
                        index,
                    })
                    .to_owned()
            })
            .collect();
        local_reservations.extend(
            canonical
                .values()
                .chain(simple_qualifiers.values())
                .filter_map(|name| name.split('.').next().map(str::to_owned)),
        );
        local_reservations.insert("java".into());
        Self {
            names,
            owner,
            simple_types: Rc::new(simple_types),
            simple_qualifiers: Rc::new(simple_qualifiers),
            canonical: Rc::new(canonical),
            imports: Rc::new(imports),
            used_imports: RefCell::new(BTreeSet::new()),
            shadowing_values: Rc::new(shadowing_values),
            shadowing_types,
            local_reservations: Rc::new(local_reservations),
        }
    }

    /// A class name in a type context, where fields and locals cannot shadow it.
    pub fn class(&self, original: &str) -> String {
        self.spelling(original, &self.simple_types)
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
        self.can_name_qualifier(original)
            .then(|| self.spelling(original, &self.simple_qualifiers))
    }

    /// Probe semantic references without requesting an unused import.
    pub(super) fn can_name_qualifier(&self, original: &str) -> bool {
        let root = self
            .simple_qualifiers
            .get(original)
            .or_else(|| self.canonical.get(original))
            .map_or_else(
                || self.names.class_name_for(original).split('/').next(),
                |name| name.split('.').next(),
            )
            .unwrap();
        !self.shadowing_values.contains(root)
            && (self.simple_qualifiers.contains_key(original)
                || !self
                    .shadowing_types
                    .iter()
                    .any(|types| types.contains(root)))
    }

    fn spelling(&self, original: &str, simple: &HashMap<String, String>) -> String {
        if simple.contains_key(original)
            && let Some(import) = self.imports.get(original)
        {
            self.used_imports.borrow_mut().insert(import.clone());
        }
        simple
            .get(original)
            .or_else(|| self.canonical.get(original))
            .cloned()
            .unwrap_or_else(|| self.names.class_name_for(original).replace('/', "."))
    }

    /// Finalized after the complete source unit has bound its type references.
    pub(super) fn imports(&self) -> BTreeSet<String> {
        self.used_imports.borrow().clone()
    }

    pub(super) fn requested_imports(&self) -> BTreeSet<String> {
        self.imports()
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
        self.simple_types.values().any(|name| name == candidate)
            || self
                .canonical
                .values()
                .any(|name| name.split('.').next() == Some(candidate))
    }

    pub(super) fn local_reservations(&self) -> &HashSet<String> {
        &self.local_reservations
    }
}

fn collect_descriptor(descriptor: &str, names: &mut BTreeSet<String>) {
    if descriptor.starts_with('(') {
        if let Ok(method) = MethodDescriptor::parse(descriptor) {
            for ty in method
                .parameters
                .iter()
                .chain(std::iter::once(&method.return_type))
            {
                collect(ty, names);
            }
        }
    } else if let Ok(ty) = JvmType::parse(descriptor) {
        collect(&ty, names);
    }
}

/// Project classes are deliberately flattened to their binary names. Library
/// classes retain real source nesting when InnerClasses metadata establishes it.
fn source_name(
    program: &Program<'_>,
    names: &NamePlan,
    original: &str,
    visiting: &mut HashSet<String>,
) -> String {
    let flat = names.class_name_for(original).replace('/', ".");
    let Some(id) = program.id(original) else {
        return flat;
    };
    if program.is_own(id) || !visiting.insert(original.to_owned()) {
        return flat;
    }
    let result = program
        .class(id)
        .inner_classes
        .iter()
        .find(|entry| entry.inner == original)
        .and_then(|entry| entry.outer.as_ref().zip(entry.inner_name.as_ref()))
        .map(|(outer, simple)| format!("{}.{simple}", source_name(program, names, outer, visiting)))
        .unwrap_or(flat);
    visiting.remove(original);
    result
}

fn collect(ty: &JvmType, names: &mut BTreeSet<String>) {
    match ty {
        JvmType::Object(name) => {
            names.insert(name.clone());
        }
        JvmType::Array(element) => collect(element, names),
        _ => {}
    }
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
            match (left_flags & 0x0200 != 0, right_flags & 0x0200 != 0) {
                (false, false) => false,
                (true, true) => interfaces_compatible(program, names, left, right),
                (true, false) => right_flags & 0x0010 == 0,
                (false, true) => left_flags & 0x0010 == 0,
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
            .filter(|&id| program.class(id).access_flags & 0x0200 != 0)
            .flat_map(|class| {
                program.class(class).methods.iter().enumerate().filter_map(
                    move |(index, method)| {
                        if method.name.starts_with('<') || method.access_flags & 0x000a != 0 {
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
