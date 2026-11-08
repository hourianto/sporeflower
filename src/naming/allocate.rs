use super::{parameters, simple_name};
use crate::Result;
use crate::classfile::{
    Constant, INTERFACE, JvmType, MethodDescriptor, NATIVE, PRIVATE, PROTECTED, PUBLIC, STATIC,
    package,
};
use crate::mappings::{MappingDiagnostic, MappingDiagnostics, MappingOrigins};
use crate::program::{ClassId, MemberId, Program};
use anyhow::{bail, ensure};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use unicode_general_category::{GeneralCategory, get_general_category};
use unicode_normalization::UnicodeNormalization;

#[path = "placement.rs"]
mod placement;

#[derive(Clone, Debug, Default)]
pub struct RenameOptions {
    /// Also replace short, apparently obfuscated class names. Legal names and
    /// package placement are otherwise retained unless Java cannot express them.
    pub rename_obfuscated: bool,
}

/// Authored names resolved against original class-file identities. Parameters
/// use declaration order, excluding `this`; wide JVM locals still count once.
#[derive(Clone, Debug, Default)]
pub struct NameOverrides {
    pub classes: HashMap<ClassId, String>,
    pub fields: HashMap<MemberId, String>,
    pub methods: HashMap<MemberId, String>,
    pub parameters: HashMap<MemberId, Vec<String>>,
    /// Source-only types, such as semantic domain interfaces.
    pub reserved_classes: HashSet<String>,
}

/// Final spellings, allocated before parallel emission. A lookup never changes
/// this plan, so source names cannot depend on worker scheduling.
#[derive(Clone, Debug)]
pub struct NamePlan {
    classes: Vec<String>,
    classes_by_original: HashMap<String, String>,
    package_types: HashMap<String, HashSet<String>>,
    java_lang_types: HashSet<String>,
    methods: Vec<Vec<String>>,
    fields: Vec<Vec<String>>,
    package_moves: Vec<(String, String)>,
    initialization_helpers: Vec<Option<String>>,
    parameters: HashMap<MemberId, Vec<String>>,
    pub diagnostics: Vec<String>,
}

impl NamePlan {
    pub fn build(program: &Program<'_>, options: &RenameOptions) -> Result<Self> {
        Self::build_with_overrides(program, options, &NameOverrides::default())
    }

    pub fn build_with_overrides(
        program: &Program<'_>,
        options: &RenameOptions,
        overrides: &NameOverrides,
    ) -> Result<Self> {
        Self::build_with_mapping_origins(program, options, overrides, &MappingOrigins::default())
    }

    /// Preserve authored locations through allocation and access validation.
    pub fn build_with_mapping_origins(
        program: &Program<'_>,
        options: &RenameOptions,
        overrides: &NameOverrides,
        origins: &MappingOrigins,
    ) -> Result<Self> {
        validate_overrides(program, overrides, origins)?;
        let mut diagnostics = Vec::new();
        let placement =
            placement::allocate(program, options, overrides, origins, &mut diagnostics)?;
        let classes = placement.classes;
        let methods = allocate_methods(program, &classes, overrides, origins, &mut diagnostics)?;
        let fields = allocate_fields(program, &classes, overrides, origins)?;
        if placement.check_boundaries {
            placement::validate_boundaries(program, &classes, &fields, origins)?;
        }
        let mut used_helper_names: HashSet<_> =
            methods.iter().chain(&fields).flatten().cloned().collect();
        used_helper_names.extend(
            classes
                .iter()
                .map(|name| name.rsplit('/').next().unwrap().to_owned()),
        );
        let mut initialization_helpers = vec![None; program.classes().len()];
        for owner in program.initialization_plan().classes() {
            let mut suffix = owner;
            let name = loop {
                let candidate = format!("$sf$initialize${suffix}");
                let container = format!("{candidate}$Trigger");
                if !used_helper_names.contains(&candidate)
                    && !used_helper_names.contains(&container)
                    && !classes
                        .iter()
                        .any(|name| *name == format!("{}${container}", classes[owner]))
                {
                    used_helper_names.insert(candidate.clone());
                    used_helper_names.insert(container);
                    break candidate;
                }
                suffix += 1;
            };
            initialization_helpers[owner] = Some(name);
        }
        let package_moves = program
            .classes()
            .iter()
            .enumerate()
            .filter(|(id, _)| program.is_own(*id))
            .filter_map(|(id, class)| {
                let original = package(&class.name);
                let emitted = package(&classes[id]);
                (original != emitted).then(|| (original.to_owned(), emitted.to_owned()))
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let classes_by_original = program
            .classes()
            .iter()
            .zip(&classes)
            .map(|(class, emitted)| (class.name.clone(), emitted.clone()))
            .collect();
        let mut package_types = HashMap::<String, HashSet<String>>::new();
        let mut java_lang_types = HashSet::new();
        for (id, emitted) in classes.iter().enumerate() {
            let package = package(emitted);
            let short = simple_name(emitted);
            package_types
                .entry(package.to_owned())
                .or_default()
                .insert(short.to_owned());
            if package == "java/lang" && program.class(id).access_flags & PUBLIC != 0 {
                java_lang_types.insert(short.to_owned());
            }
        }
        Ok(Self {
            classes,
            classes_by_original,
            package_types,
            java_lang_types,
            methods,
            fields,
            package_moves,
            initialization_helpers,
            parameters: overrides.parameters.clone(),
            diagnostics,
        })
    }

    pub fn class_name(&self, id: ClassId) -> &str {
        &self.classes[id]
    }
    /// Source types visible without imports, after final package/name allocation.
    pub(crate) fn visible_package_types(&self, owner: ClassId) -> [&HashSet<String>; 2] {
        [
            &self.package_types[package(self.class_name(owner))],
            &self.java_lang_types,
        ]
    }
    pub fn class_name_for<'a>(&'a self, original: &'a str) -> &'a str {
        self.classes_by_original
            .get(original)
            .map_or(original, String::as_str)
    }
    /// Original and emitted internal names, including unchanged library types.
    pub fn class_mappings(&self) -> impl Iterator<Item = (&str, &str)> {
        self.classes_by_original
            .iter()
            .map(|(original, emitted)| (original.as_str(), emitted.as_str()))
    }
    pub fn method_name(&self, id: MemberId) -> &str {
        &self.methods[id.class][id.index]
    }
    pub fn field_name(&self, id: MemberId) -> &str {
        &self.fields[id.class][id.index]
    }
    pub(crate) fn source_field_resolves(
        &self,
        program: &Program<'_>,
        owner: ClassId,
        name: &str,
        target: MemberId,
    ) -> bool {
        placement::source_field_resolves(program, &self.fields, owner, name, target)
    }
    pub fn initialization_helper(&self, id: ClassId) -> Option<&str> {
        self.initialization_helpers
            .get(id)
            .and_then(Option::as_deref)
    }

    pub fn parameter_names(&self, id: MemberId) -> Option<&[String]> {
        self.parameters.get(&id).map(Vec::as_slice)
    }

    pub fn parameter_name(&self, id: MemberId, ordinal: usize) -> Option<&str> {
        self.parameter_names(id)
            .and_then(|names| names.get(ordinal))
            .map(String::as_str)
    }

    /// Original and emitted package identities, in stable original-name order.
    pub fn package_moves(&self) -> &[(String, String)] {
        &self.package_moves
    }

    pub fn resolved_method_name<'a>(
        &'a self,
        program: &Program<'_>,
        owner: &str,
        name: &'a str,
        descriptor: &str,
    ) -> &'a str {
        program
            .resolve_method(owner, name, descriptor)
            .map_or(name, |id| self.method_name(id))
    }
    pub fn resolved_field_name<'a>(
        &'a self,
        program: &Program<'_>,
        owner: &str,
        name: &'a str,
        descriptor: &str,
    ) -> &'a str {
        program
            .resolve_field(owner, name, descriptor)
            .map_or(name, |id| self.field_name(id))
    }

    pub fn type_name(&self, ty: &JvmType) -> String {
        match ty {
            JvmType::Void => "void".into(),
            JvmType::Boolean => "boolean".into(),
            JvmType::Byte => "byte".into(),
            JvmType::Char => "char".into(),
            JvmType::Short => "short".into(),
            JvmType::Int => "int".into(),
            JvmType::Long => "long".into(),
            JvmType::Float => "float".into(),
            JvmType::Double => "double".into(),
            JvmType::Object(name) => self.class_name_for(name).replace('/', "."),
            JvmType::Array(element) => format!("{}[]", self.type_name(element)),
        }
    }
}

/// Java identifier spelling excluding ignorable code points. Categories follow
/// Java's letter, letter-number, currency, connector, mark, and decimal rules.
/// Ignorable characters are never retained: javac would silently erase them.
pub fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let start = |ch: char| {
        matches!(
            get_general_category(ch),
            GeneralCategory::UppercaseLetter
                | GeneralCategory::LowercaseLetter
                | GeneralCategory::TitlecaseLetter
                | GeneralCategory::ModifierLetter
                | GeneralCategory::OtherLetter
                | GeneralCategory::LetterNumber
                | GeneralCategory::CurrencySymbol
                | GeneralCategory::ConnectorPunctuation
        )
    };
    let part = |ch: char| {
        start(ch)
            || matches!(
                get_general_category(ch),
                GeneralCategory::DecimalNumber
                    | GeneralCategory::SpacingMark
                    | GeneralCategory::NonspacingMark
            )
    };
    if !start(first) || !chars.all(part) {
        return false;
    }
    !matches!(
        name,
        "abstract"
            | "assert"
            | "boolean"
            | "break"
            | "byte"
            | "case"
            | "catch"
            | "char"
            | "class"
            | "const"
            | "continue"
            | "default"
            | "do"
            | "double"
            | "else"
            | "enum"
            | "extends"
            | "final"
            | "finally"
            | "float"
            | "for"
            | "goto"
            | "if"
            | "implements"
            | "import"
            | "instanceof"
            | "int"
            | "interface"
            | "long"
            | "native"
            | "new"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "return"
            | "short"
            | "static"
            | "strictfp"
            | "super"
            | "switch"
            | "synchronized"
            | "this"
            | "throw"
            | "throws"
            | "transient"
            | "try"
            | "void"
            | "volatile"
            | "while"
            | "true"
            | "false"
            | "null"
            | "_"
    )
}

fn portable_segment(name: &str) -> bool {
    if !is_identifier(name) || name.len() > 240 {
        return false;
    }
    !matches!(
        name.to_ascii_lowercase().as_str(),
        "con"
            | "prn"
            | "aux"
            | "nul"
            | "com1"
            | "com2"
            | "com3"
            | "com4"
            | "com5"
            | "com6"
            | "com7"
            | "com8"
            | "com9"
            | "lpt1"
            | "lpt2"
            | "lpt3"
            | "lpt4"
            | "lpt5"
            | "lpt6"
            | "lpt7"
            | "lpt8"
            | "lpt9"
    )
}

fn path_key(path: &str) -> String {
    path.nfc().flat_map(char::to_lowercase).nfc().collect()
}

fn join(package: &str, simple: &str) -> String {
    if package.is_empty() {
        simple.to_owned()
    } else {
        format!("{package}/{simple}")
    }
}

fn validate_overrides(
    program: &Program<'_>,
    overrides: &NameOverrides,
    origins: &MappingOrigins,
) -> Result<()> {
    let mut errors = MappingDiagnostics::default();
    let mut classes: Vec<_> = overrides.classes.iter().collect();
    classes.sort_by_key(|(id, _)| **id);
    for (&id, name) in classes {
        let result = (|| -> Result<()> {
            let Some(class) = program.classes().get(id) else {
                bail!("class mapping refers to unknown class ID {id}");
            };
            ensure!(
                program.is_own(id) || name == &class.name,
                "class mapping cannot rename external class {} to {name}",
                class.name
            );
            ensure!(
                name.split('/').all(portable_segment),
                "invalid authored class name {name} for {}",
                class.name
            );
            Ok(())
        })();
        if let Err(error) = result {
            errors.push(attach_origin(error, origins.classes.get(&id)));
        }
    }
    let mut reserved: Vec<_> = overrides.reserved_classes.iter().collect();
    reserved.sort();
    for name in reserved {
        let result = (|| -> Result<()> {
            ensure!(
                name.split('/').all(portable_segment),
                "invalid source-only class name {name}"
            );
            Ok(())
        })();
        if let Err(error) = result {
            errors.push(attach_origin(error, origins.reserved_classes.get(name)));
        }
    }
    for (kind, members, member_origins) in [
        ("method", &overrides.methods, &origins.methods),
        ("field", &overrides.fields, &origins.fields),
    ] {
        let mut members: Vec<_> = members.iter().collect();
        members.sort_by_key(|(id, _)| **id);
        for (&id, name) in members {
            let result = (|| -> Result<()> {
                let Some(class) = program.classes().get(id.class) else {
                    bail!("{kind} mapping refers to unknown class ID {}", id.class);
                };
                let original = if kind == "method" {
                    class.methods.get(id.index).map(|method| &method.name)
                } else {
                    class.fields.get(id.index).map(|field| &field.name)
                };
                let Some(original) = original else {
                    bail!(
                        "{kind} mapping refers to unknown member {}:{}",
                        class.name,
                        id.index
                    );
                };
                ensure!(
                    program.is_own(id.class) || name == original,
                    "{kind} mapping cannot rename external member {}.{original} to {name}",
                    class.name
                );
                ensure!(
                    is_source_member(name)
                        || (kind == "method" && original.starts_with('<') && name == original),
                    "invalid authored {kind} name {name} for {}.{original}",
                    class.name
                );
                Ok(())
            })();
            if let Err(error) = result {
                errors.push(attach_origin(error, member_origins.get(&id)));
            }
        }
    }
    let mut parameters: Vec<_> = overrides.parameters.iter().collect();
    parameters.sort_by_key(|(id, _)| **id);
    for (&id, names) in parameters {
        let result = (|| -> Result<()> {
            let Some(method) = program
                .classes()
                .get(id.class)
                .and_then(|class| class.methods.get(id.index))
            else {
                bail!(
                    "parameter mapping refers to unknown method {}:{}",
                    id.class,
                    id.index
                );
            };
            ensure!(
                names.len()
                    == MethodDescriptor::parse(&method.descriptor)?
                        .parameters
                        .len(),
                "authored parameter count does not match {}.{}{}",
                program.class(id.class).name,
                method.name,
                method.descriptor
            );
            let mut used = HashSet::new();
            for name in names {
                ensure!(
                    is_source_parameter(name),
                    "invalid authored parameter name {name} for {}.{}{}",
                    program.class(id.class).name,
                    method.name,
                    method.descriptor
                );
                ensure!(
                    used.insert(name),
                    "duplicate authored parameter name {name} for {}.{}{}",
                    program.class(id.class).name,
                    method.name,
                    method.descriptor
                );
            }
            Ok(())
        })();
        if let Err(error) = result {
            errors.push(attach_origin(error, origins.parameters.get(&id)));
        }
    }
    errors.finish()
}

fn attach_origin(
    error: anyhow::Error,
    origin: Option<&crate::mappings::MappingOrigin>,
) -> anyhow::Error {
    match origin {
        Some(origin) => origin.attach(error),
        None => error,
    }
}

/// Shared by authored and advisory local names. Renderer temporaries occupy
/// these prefixes even when a particular method has not needed one yet.
pub(crate) fn is_source_parameter(name: &str) -> bool {
    is_source_member(name)
        && ![
            "$sf$",
            "$state",
            "$arguments",
            "$caught",
            "$checked",
            "$CheckedThrow",
            "$exception",
            "$discard",
            "$marker",
            "$Preinit",
            "$preinit",
            "$Init",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
        && !matches!(name, "$initializer" | "$initialized" | "$unused")
}

fn package_prefixes(name: &str) -> impl Iterator<Item = &str> {
    std::iter::successors((!name.is_empty()).then_some(name), |name| {
        let parent = package(name);
        (!parent.is_empty()).then_some(parent)
    })
}

fn named_code_uses_default_package(program: &Program<'_>, emitted: Option<&[String]>) -> bool {
    let source_name =
        |id| emitted.map_or(program.class(id).name.as_str(), |names| names[id].as_str());
    let default_type = |name: &str| {
        program
            .id(name)
            .is_some_and(|id| program.is_own(id) && package(source_name(id)).is_empty())
    };
    fn type_uses(ty: &JvmType, check: &impl Fn(&str) -> bool) -> bool {
        match ty {
            JvmType::Object(name) => check(name),
            JvmType::Array(element) => type_uses(element, check),
            _ => false,
        }
    }
    let descriptor_uses = |descriptor: &str| {
        if descriptor.starts_with('(') {
            crate::classfile::MethodDescriptor::parse(descriptor).is_ok_and(|method| {
                method
                    .parameters
                    .iter()
                    .chain(std::iter::once(&method.return_type))
                    .any(|ty| type_uses(ty, &default_type))
            })
        } else {
            JvmType::parse(descriptor).is_ok_and(|ty| type_uses(&ty, &default_type))
        }
    };
    (0..program.own_count())
        .filter(|&owner| !package(source_name(owner)).is_empty())
        .any(|owner| {
            let class = program.class(owner);
            class
                .super_name
                .iter()
                .chain(&class.interfaces)
                .any(|name| default_type(name))
                || class
                    .fields
                    .iter()
                    .any(|field| descriptor_uses(&field.descriptor))
                || class
                    .methods
                    .iter()
                    .any(|method| descriptor_uses(&method.descriptor))
                || class
                    .constant_pool
                    .entries
                    .iter()
                    .any(|constant| match constant {
                        Constant::Class(index) => {
                            class.constant_pool.utf8(*index).is_ok_and(|name| {
                                if name.starts_with('[') {
                                    descriptor_uses(name)
                                } else {
                                    default_type(name)
                                }
                            })
                        }
                        Constant::NameAndType {
                            descriptor_index, ..
                        } => class
                            .constant_pool
                            .utf8(*descriptor_index)
                            .is_ok_and(descriptor_uses),
                        _ => false,
                    })
        })
}

fn allocate_classes(
    program: &Program<'_>,
    options: &RenameOptions,
    diagnostics: &mut Vec<String>,
) -> Vec<String> {
    let mut packages = BTreeSet::new();
    for class in program.classes() {
        let mut prefix = package(&class.name);
        while !prefix.is_empty() {
            packages.insert(prefix.to_owned());
            prefix = package(prefix);
        }
    }
    let mut package_names = HashMap::from([(String::new(), String::new())]);
    let mut reserved: HashSet<_> = packages.iter().map(|p| path_key(p)).collect();
    let mut occupied_packages = HashSet::new();
    if named_code_uses_default_package(program, None) {
        let original_roots: HashSet<_> = program
            .classes()
            .iter()
            .map(|class| path_key(class.name.split('/').next().unwrap()))
            .collect();
        let mut emitted = "defaultpkg".to_owned();
        let mut suffix = 0;
        while original_roots.contains(&path_key(&emitted)) {
            emitted = format!("defaultpkg_{suffix}");
            suffix += 1;
        }
        occupied_packages.insert(path_key(&emitted));
        reserved.insert(path_key(&emitted));
        package_names.insert(String::new(), emitted);
    }
    let mut counter = 0;
    // Lexical ordering visits a prefix before its descendants.
    for original in &packages {
        // A named package is rooted at the source root; relocating the unnamed
        // package must not accidentally prepend that destination to named peers.
        let parent = if package(original).is_empty() {
            ""
        } else {
            package_names
                .get(package(original))
                .expect("package prefix planned")
                .as_str()
        };
        let simple = simple_name(original);
        let desired = join(parent, simple);
        let fixed = program.classes().iter().enumerate().any(|(id, class)| {
            !program.is_own(id)
                && (package(&class.name) == original
                    || package(&class.name).starts_with(&format!("{original}/")))
        });
        let mut emitted = desired;
        if !fixed && (!portable_segment(simple) || occupied_packages.contains(&path_key(&emitted)))
        {
            loop {
                emitted = join(parent, &format!("package_{counter}"));
                counter += 1;
                if !reserved.contains(&path_key(&emitted))
                    && !occupied_packages.contains(&path_key(&emitted))
                {
                    break;
                }
            }
        }
        if fixed && !portable_segment(simple) {
            diagnostics.push(format!(
                "external package {original} has no legal Java spelling"
            ));
        }
        occupied_packages.insert(path_key(&emitted));
        reserved.insert(path_key(&emitted));
        package_names.insert(original.clone(), emitted);
    }
    let desired: Vec<_> = program
        .classes()
        .iter()
        .map(|class| {
            join(
                &package_names[package(&class.name)],
                simple_name(&class.name),
            )
        })
        .collect();
    let reserved_classes: HashSet<_> = desired.iter().map(|name| path_key(name)).collect();
    // A visible type named like a package root obscures that root even in a
    // fully qualified Java type. Reserve roots in the type namespace first.
    let package_roots: HashSet<_> = package_names
        .values()
        .filter(|name| !name.is_empty())
        .map(|name| name.split('/').next().unwrap().to_owned())
        .collect();
    let mut occupied: HashSet<_> = program
        .classes()
        .iter()
        .enumerate()
        .filter(|(id, _)| !program.is_own(*id))
        .map(|(_, class)| path_key(&class.name))
        .collect();
    let mut names = Vec::with_capacity(desired.len());
    let mut class_counter = 0;
    for (id, mut candidate) in desired.into_iter().enumerate() {
        let class = program.class(id);
        if !program.is_own(id) {
            names.push(class.name.clone());
            continue;
        }
        let simple = simple_name(&class.name);
        let short =
            options.rename_obfuscated && simple.split('$').all(|part| part.chars().count() <= 3);
        if !portable_segment(simple)
            || package_roots.contains(simple)
            || short
            || occupied.contains(&path_key(&candidate))
            || occupied_packages.contains(&path_key(&candidate))
        {
            loop {
                candidate = join(
                    &package_names[package(&class.name)],
                    &format!("class_{class_counter}"),
                );
                class_counter += 1;
                let key = path_key(&candidate);
                if !occupied.contains(&key)
                    && !occupied_packages.contains(&key)
                    && !reserved_classes.contains(&key)
                    && !package_roots.contains(simple_name(&candidate))
                {
                    break;
                }
            }
        }
        occupied.insert(path_key(&candidate));
        names.push(candidate);
    }
    names
}

struct UnionFind {
    parent: Vec<usize>,
}
impl UnionFind {
    fn new(count: usize) -> Self {
        Self {
            parent: (0..count).collect(),
        }
    }
    fn find(&mut self, mut id: usize) -> usize {
        while self.parent[id] != id {
            self.parent[id] = self.parent[self.parent[id]];
            id = self.parent[id];
        }
        id
    }
    fn union(&mut self, a: usize, b: usize) {
        let a = self.find(a);
        let b = self.find(b);
        if a != b {
            self.parent[a.max(b)] = a.min(b);
        }
    }
}

struct Family {
    members: Vec<MemberId>,
    scopes: Vec<usize>,
    name: String,
    fixed: bool,
    authored: bool,
}

fn allocate_methods(
    program: &Program<'_>,
    classes: &[String],
    overrides: &NameOverrides,
    origins: &MappingOrigins,
    diagnostics: &mut Vec<String>,
) -> Result<Vec<Vec<String>>> {
    let mut offsets = Vec::with_capacity(program.classes().len());
    let mut members = Vec::new();
    for (class, definition) in program.classes().iter().enumerate() {
        offsets.push(members.len());
        members.extend((0..definition.methods.len()).map(|index| MemberId { class, index }));
    }
    let mut union = UnionFind::new(members.len());
    let mut scopes = Vec::<Vec<usize>>::new();
    for owner in 0..program.own_count() {
        let mut identities: HashMap<(&str, &str), usize> = HashMap::new();
        let mut source_scopes: HashMap<&str, Vec<usize>> = HashMap::new();
        for class in std::iter::once(owner).chain(program.ancestors(owner).iter().copied()) {
            for (index, method) in program.class(class).methods.iter().enumerate() {
                if method.name.starts_with('<') {
                    continue;
                }
                if class != owner
                    && program.class(class).access_flags & INTERFACE != 0
                    && method.access_flags & STATIC != 0
                {
                    continue;
                }
                let member = offsets[class] + index;
                // Java collision scopes follow final packages, while override
                // families remain identities in the original bytecode graph.
                let source_visible = class == owner
                    || (method.access_flags & PRIVATE == 0
                        && (method.access_flags & (PUBLIC | PROTECTED) != 0
                            || package(&classes[class]) == package(&classes[owner])));
                if source_visible {
                    source_scopes
                        .entry(parameters(&method.descriptor))
                        .or_default()
                        .push(member);
                }
                if method.access_flags & (PRIVATE | STATIC) != 0
                    || !program.visible_from(method.access_flags, class, owner)
                {
                    continue;
                }
                let identity = (method.name.as_str(), method.descriptor.as_str());
                if let Some(&previous) = identities.get(&identity) {
                    union.union(member, previous);
                } else {
                    identities.insert(identity, member);
                }
            }
        }
        let mut entries: Vec<_> = source_scopes.into_iter().collect();
        entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
        scopes.extend(entries.into_iter().map(|(_, members)| members));
    }
    let mut families: Vec<Option<Family>> = (0..members.len()).map(|_| None).collect();
    for (id, &member) in members.iter().enumerate() {
        let method = program.method(member);
        let root = union.find(id);
        let family = families[root].get_or_insert_with(|| Family {
            members: Vec::new(),
            scopes: Vec::new(),
            name: method.name.clone(),
            fixed: false,
            authored: false,
        });
        family.members.push(member);
        family.fixed |= !program.is_own(member.class)
            || method.access_flags & NATIVE != 0
            || method.name.starts_with('<');
    }
    for family in families.iter_mut().flatten() {
        for member in &family.members {
            if let Some(name) = overrides.methods.get(member) {
                ensure!(
                    !family.authored || name == &family.name,
                    MappingDiagnostic::with_origins(
                        format!(
                            "conflicting authored names for override family {}.{}{}: {} and {name}",
                            program.class(member.class).name,
                            program.method(*member).name,
                            program.method(*member).descriptor,
                            family.name
                        ),
                        family
                            .members
                            .iter()
                            .filter_map(|member| origins.methods.get(member).cloned())
                    )
                    .hint("give every mapped declaration in this override family the same name")
                );
                if family.fixed {
                    ensure!(
                        name == &program.method(*member).name,
                        MappingDiagnostic::with_origins(format!("authored method name {name} changes a fixed library, native or constructor contract: {}.{}{}",
                        program.class(member.class).name,
                        program.method(*member).name,
                        program.method(*member).descriptor
                        ), origins.methods.get(member).cloned()).hint("preserve the original name of library, native and constructor contracts")
                    );
                }
                family.name.clone_from(name);
                family.authored = true;
            }
        }
    }
    for (scope, members) in scopes.iter().enumerate() {
        let mut roots: Vec<_> = members.iter().map(|&id| union.find(id)).collect();
        roots.sort_unstable();
        roots.dedup();
        for root in roots {
            families[root].as_mut().unwrap().scopes.push(scope);
        }
    }
    let mut order: Vec<_> = families
        .iter()
        .enumerate()
        .filter_map(|(id, f)| f.as_ref().map(|_| id))
        .collect();
    order.sort_by_key(|&id| {
        let f = families[id].as_ref().unwrap();
        (!f.fixed, !f.authored, !is_source_member(&f.name), id)
    });
    let mut occupied = vec![HashMap::<String, usize>::new(); scopes.len()];
    let reserved: HashSet<_> = members
        .iter()
        .map(|&member| program.method(member).name.as_str())
        .chain(overrides.methods.values().map(String::as_str))
        .collect();
    let mut counter = 0;
    for root in order {
        let family = families[root].as_ref().unwrap();
        let mut name = family.name.clone();
        let collides = |candidate: &str, occupied: &[HashMap<String, usize>]| {
            family
                .scopes
                .iter()
                .any(|&scope| occupied[scope].contains_key(candidate))
        };
        if family.authored && !name.starts_with('<') && collides(&name, &occupied) {
            let member = family.members[0];
            let conflicts = family
                .scopes
                .iter()
                .filter_map(|&scope| occupied[scope].get(&name))
                .flat_map(|&root| families[root].as_ref().unwrap().members.iter());
            bail!(MappingDiagnostic::with_origins(format!(
                "authored method name {name} collides in Java for {}.{}{}",
                program.class(member.class).name,
                program.method(member).name,
                program.method(member).descriptor
            ), family.members.iter().chain(conflicts).filter_map(|member| origins.methods.get(member).cloned()))
                .hint("choose distinct names for methods with the same Java parameter types; return types do not distinguish overloads"));
        }
        if !family.fixed
            && !family.authored
            && (!is_source_member(&name) || collides(&name, &occupied))
        {
            loop {
                name = format!("method_{counter}");
                counter += 1;
                if !reserved.contains(name.as_str()) && !collides(&name, &occupied) {
                    break;
                }
            }
        } else if family.fixed
            && !name.starts_with('<')
            && (!is_source_member(&name) || collides(&name, &occupied))
            && family
                .members
                .iter()
                .any(|member| program.is_own(member.class))
        {
            let member = family.members[0];
            diagnostics.push(format!(
                "fixed method contract {}.{}{} cannot be expressed without a Java name conflict",
                program.class(member.class).name,
                name,
                program.method(member).descriptor
            ));
        }
        for &scope in &family.scopes {
            occupied[scope].insert(name.clone(), root);
        }
        families[root].as_mut().unwrap().name = name;
    }
    Ok(program
        .classes()
        .iter()
        .enumerate()
        .map(|(class, definition)| {
            (0..definition.methods.len())
                .map(|index| {
                    families[union.find(offsets[class] + index)]
                        .as_ref()
                        .unwrap()
                        .name
                        .clone()
                })
                .collect()
        })
        .collect())
}

fn is_source_member(name: &str) -> bool {
    // Legacy javac reserves this prefix for enclosing-instance members.
    // Flattened binary inner classes retain their state under legal field names.
    is_identifier(name) && !name.starts_with("this$") && name != "class$"
}

fn allocate_fields(
    program: &Program<'_>,
    classes: &[String],
    overrides: &NameOverrides,
    origins: &MappingOrigins,
) -> Result<Vec<Vec<String>>> {
    let mut names: Vec<Vec<String>> = program
        .classes()
        .iter()
        .map(|class| class.fields.iter().map(|f| f.name.clone()).collect())
        .collect();
    let reserved: HashSet<_> = program
        .classes()
        .iter()
        .flat_map(|class| class.fields.iter().map(|field| field.name.as_str()))
        .chain(overrides.fields.values().map(String::as_str))
        .collect();
    // Fully qualified type expressions must not be obscured by a field with the
    // package root's name. Default-package types have no qualified alternative.
    let mut qualifier_roots = field_qualifier_roots(program, classes);
    for roots in &mut qualifier_roots {
        roots.extend(
            overrides
                .reserved_classes
                .iter()
                .map(|name| name.split('/').next().unwrap().to_owned()),
        );
    }
    let mut inherited_roots = qualifier_roots.clone();
    for (owner, roots) in qualifier_roots.iter().enumerate().take(program.own_count()) {
        for &ancestor in program.ancestors(owner) {
            if program.is_own(ancestor) {
                inherited_roots[ancestor].extend(roots.iter().cloned());
            }
        }
    }
    let mut counter = 0;
    // Parent spellings are stable before a child is allocated. JVM fields are
    // identified by name *and descriptor*, while Java selects only by name.
    // A differently typed child field can otherwise capture an inherited field
    // reference through an accessible child of an inaccessible declaring class.
    let mut owners: Vec<_> = (0..program.own_count()).collect();
    owners.sort_by_key(|&owner| (program.ancestors(owner).len(), owner));
    for owner in owners {
        let mut occupied = HashSet::new();
        let mut inherited: HashMap<String, HashSet<(&str, &str)>> = HashMap::new();
        for &ancestor in program.ancestors(owner) {
            for (index, field) in program.class(ancestor).fields.iter().enumerate() {
                if field.access_flags & PRIVATE == 0 {
                    inherited
                        .entry(names[ancestor][index].clone())
                        .or_default()
                        .insert((&field.name, &field.descriptor));
                }
            }
        }
        let mut indices: Vec<_> = (0..program.class(owner).fields.len()).collect();
        indices.sort_by_key(|&index| {
            (
                !overrides.fields.contains_key(&MemberId {
                    class: owner,
                    index,
                }),
                index,
            )
        });
        for index in indices {
            let field = &program.class(owner).fields[index];
            let member = MemberId {
                class: owner,
                index,
            };
            let authored = overrides.fields.get(&member);
            let qualifier_roots = &inherited_roots[owner];
            let mut name = authored.unwrap_or(&field.name).clone();
            // A child's authored name must not force an already allocated
            // automatic ancestor field to capture a differently typed access.
            let descendant_conflict = |candidate: &str| {
                field.access_flags & PRIVATE == 0
                    && overrides.fields.iter().any(|(child, target)| {
                        target == candidate
                            && program.ancestors(child.class).contains(&owner)
                            && (program.field(*child).descriptor != field.descriptor
                                || program.field(*child).name != field.name)
                    })
            };
            let conflict = !is_source_member(&name)
                || occupied.contains(&name)
                || qualifier_roots.contains(&name)
                || inherited.get(&name).is_some_and(|identities| {
                    identities
                        .iter()
                        .any(|(original, ty)| *ty != field.descriptor || *original != field.name)
                })
                || descendant_conflict(&name);
            ensure!(
                authored.is_none() || !conflict,
                MappingDiagnostic::with_origins(
                    format!(
                        "authored field name {name} collides in Java for {}.{}:{}",
                        program.class(owner).name,
                        field.name,
                        field.descriptor
                    ),
                    overrides
                        .fields
                        .iter()
                        .filter(|(id, target)| **target == name
                            && (id.class == owner
                                || program.ancestors(owner).contains(&id.class)
                                || program.ancestors(id.class).contains(&owner)))
                        .filter_map(|(id, _)| origins.fields.get(id).cloned())
                )
                .hint("choose distinct mapped field names in this class hierarchy")
            );
            if conflict {
                loop {
                    name = format!("field_{counter}");
                    counter += 1;
                    if !reserved.contains(name.as_str())
                        && !occupied.contains(&name)
                        && !qualifier_roots.contains(&name)
                        && !inherited.contains_key(&name)
                    {
                        break;
                    }
                }
            }
            occupied.insert(name.clone());
            names[owner][index] = name;
        }
    }
    Ok(names)
}

/// Reserve only qualifiers which occur in an expression in the affected source
/// scope. Reserving every project class globally renamed almost every short
/// field in obfuscated default-package applications, even in unrelated classes.
fn field_qualifier_roots(program: &Program<'_>, classes: &[String]) -> Vec<HashSet<String>> {
    let mut roots = vec![HashSet::from(["java".to_owned()]); program.own_count()];
    for (owner, roots) in roots.iter_mut().enumerate() {
        let class = program.class(owner);
        // Source-only helpers can introduce static self calls and explicit NEW
        // initialization triggers absent from the original constant pool.
        roots.insert(classes[owner].split('/').next().unwrap().to_owned());
        for index in 0..class.methods.len() {
            for site in program.initialization_plan().sites(MemberId {
                class: owner,
                index,
            }) {
                let emitted = program
                    .id(&site.class)
                    .map_or(site.class.as_str(), |id| classes[id].as_str());
                roots.insert(emitted.split('/').next().unwrap().to_owned());
            }
        }
        for (index, constant) in class.constant_pool.entries.iter().enumerate() {
            let target = match constant {
                Constant::Fieldref { .. } => {
                    let Ok(member) = class.constant_pool.member_ref(index as u16) else {
                        continue;
                    };
                    program
                        .resolve_field(member.owner, member.name, member.descriptor)
                        .map(|id| (id.class, program.field(id).access_flags))
                }
                Constant::Methodref { .. } | Constant::InterfaceMethodref { .. } => {
                    let Ok(member) = class.constant_pool.member_ref(index as u16) else {
                        continue;
                    };
                    program
                        .resolve_method(member.owner, member.name, member.descriptor)
                        .map(|id| (id.class, program.method(id).access_flags))
                }
                _ => continue,
            };
            if target.is_some_and(|(_, flags)| flags & STATIC == 0) {
                continue;
            }
            let Ok(member) = class.constant_pool.member_ref(index as u16) else {
                continue;
            };
            let emitted = program
                .id(member.owner)
                .map_or(member.owner, |id| classes[id].as_str());
            roots.insert(emitted.split('/').next().unwrap().to_owned());
            if let Some((declaration, _)) = target {
                roots.insert(classes[declaration].split('/').next().unwrap().to_owned());
            }
        }
    }
    roots
}
