use super::{
    parser::{Annotation, Declaration, Member, ParsedSource, Type, Value},
    *,
};
use crate::hash::{HashMap, HashSet};
use crate::{
    Result,
    classfile::{Constant, JvmType, MethodDescriptor},
    naming::is_identifier,
    program::{ClassId, MemberId, Program},
};
use anyhow::{Context, bail, ensure};

/// Unfinished declarations own their provenance until the validated plan is frozen.
pub(crate) struct Resolver<'a, 'p> {
    pub program: &'a Program<'p>,
    pub names: crate::naming::NameOverrides,
    pub origins: MappingOrigins,
    pub diagnostics: Vec<MappingDiagnostic>,
    pub(super) domains: Vec<PendingDomain>,
    pub domain_names: HashMap<String, DomainId>,
    classes: HashMap<String, ClassId>,
    known_types: HashSet<String>,
    pub(super) bindings: HashMap<Target, Binding<Contract>>,
    real_values: HashMap<MemberId, Binding<DomainId>>,
    calls: HashMap<CallSite, Binding<DomainId>>,
    ignored_classes: HashSet<ClassId>,
}

pub(super) struct PendingDomain {
    pub origin: MappingOrigin,
    pub owner: String,
    pub kind: DomainKind,
    pub values: Vec<(DomainValue, MappingOrigin)>,
    pub exclusive_masks: Vec<i64>,
    pub bit_fields: Vec<BitField>,
    pub format: Option<NumberFormat>,
}

impl PendingDomain {
    fn freeze(self) -> Domain {
        Domain {
            owner: self.owner,
            kind: self.kind,
            values: self.values.into_iter().map(|(value, _)| value).collect(),
            exclusive_masks: self.exclusive_masks,
            bit_fields: self.bit_fields,
            format: self.format,
        }
    }
    fn generated(&self) -> bool {
        self.values.iter().any(|(value, _)| value.source.is_none())
            || self.bit_fields.iter().any(|field| field.name.is_some())
    }
}

#[derive(Clone)]
pub(super) struct Binding<T> {
    pub value: T,
    pub origin: MappingOrigin,
    pub builtin: bool,
}

impl<T> Binding<T> {
    pub(super) fn new(source: &ParsedSource, origin: &MappingOrigin, value: T) -> Self {
        Self {
            value,
            origin: origin.clone(),
            builtin: source.builtin,
        }
    }
}

pub(super) enum BindingKind {
    Contract,
    Value,
    Call,
}

/// All explicit semantic bindings obey the same authority and duplicate policy.
pub(super) fn insert_binding<K: Eq + std::hash::Hash, V: PartialEq>(
    bindings: &mut HashMap<K, Binding<V>>,
    diagnostics: &mut Vec<MappingDiagnostic>,
    key: K,
    binding: Binding<V>,
    kind: BindingKind,
) -> Result<bool> {
    if let Some(previous) = bindings.get(&key) {
        let (duplicate, shadowed) = match kind {
            BindingKind::Contract => ("semantic target binding", "contract"),
            BindingKind::Value => ("@DomainValue binding", "@DomainValue binding"),
            BindingKind::Call => ("scoped call contract", "scoped call contract"),
        };
        let origins = [previous.origin.clone(), binding.origin.clone()];
        if previous.builtin && !binding.builtin {
            if previous.value != binding.value {
                diagnostics.push(MappingDiagnostic::with_origins(
                    format!("authored {shadowed} is shadowed by a builtin API contract"), origins,
                ).warning().hint("remove this annotation, or use --no-builtin-mappings to supply your own API contracts"));
            }
            return Ok(false);
        }
        let mut issue = MappingDiagnostic::with_origins(format!("duplicate {duplicate}"), origins);
        if matches!(kind, BindingKind::Contract) {
            issue = issue.hint("combine the annotations into one declaration of this target");
        }
        bail!(issue);
    }
    bindings.insert(key, binding);
    Ok(true)
}

pub(crate) fn resolve(program: &Program<'_>, sources: &[ParsedSource]) -> Result<MappingPlan> {
    let mut known_types = HashSet::default();
    for class in program.classes() {
        known_types.insert(class.name.clone());
        known_types.extend(class.super_name.iter().cloned());
        known_types.extend(class.interfaces.iter().cloned());
        for field in &class.fields {
            collect_type(&JvmType::parse(&field.descriptor)?, &mut known_types);
        }
        for method in &class.methods {
            let descriptor = MethodDescriptor::parse(&method.descriptor)?;
            collect_type(&descriptor.return_type, &mut known_types);
            for parameter in descriptor.parameters {
                collect_type(&parameter, &mut known_types);
            }
        }
    }
    let mut resolver = Resolver {
        program,
        names: Default::default(),
        origins: MappingOrigins::default(),
        diagnostics: Vec::new(),
        domains: Vec::new(),
        bindings: HashMap::default(),
        real_values: HashMap::default(),
        calls: HashMap::default(),
        ignored_classes: HashSet::default(),
        domain_names: HashMap::default(),
        classes: HashMap::default(),
        known_types,
    };
    // Finish each dependency phase before proceeding, so an invalid owner or
    // domain cannot produce a cascade of missing members or secondary contracts.
    let mut errors = MappingDiagnostics::default();
    for source in sources {
        for declaration in &source.declarations {
            if let Err(error) = resolver.register(source, declaration) {
                errors.push(declaration.origin.attach(error));
            }
        }
    }
    errors.finish()?;
    let mut errors = MappingDiagnostics::default();
    for source in sources {
        for declaration in &source.declarations {
            if let Some(kind) = domain_kind(&declaration.annotations)?
                && let Err(error) = resolver.domain_declaration(source, declaration, kind)
            {
                errors.push(declaration.origin.attach(error));
            }
        }
    }
    errors.finish()?;
    let mut errors = MappingDiagnostics::default();
    for source in sources {
        for declaration in &source.declarations {
            if domain_kind(&declaration.annotations)?.is_some()
                || has(&declaration.annotations, "AlreadyMapped")
            {
                continue;
            }
            let owner = resolver.classes[&qualified(&source.package, &declaration.name)];
            for member in &declaration.members {
                if let Err(error) = resolver.member(source, declaration, owner, member) {
                    errors.push(member.origin.attach(error));
                }
            }
        }
    }
    errors.finish()?;
    // Legacy maps use an omitted package as the shared readable `defpackage`
    // namespace. Normalize only authored output names, after every original
    // identity and descriptor has been resolved. Native NameOverrides remain
    // exact, and external API owners never enter the authored class table.
    let mut emitted_classes = HashMap::default();
    for (id, target) in &mut resolver.names.classes {
        if !target.contains('/') {
            *target = format!("defpackage/{target}");
        }
        if let Some(previous) = emitted_classes.insert(target.clone(), *id) {
            ensure!(
                previous == *id,
                MappingDiagnostic::with_origins(
                    format!("duplicate mapped class target {target}"),
                    [*id, previous].into_iter().filter_map(|id| resolver
                        .origins
                        .classes
                        .get(&id)
                        .cloned())
                )
                .hint("choose distinct class targets; an omitted package means defpackage")
            );
        }
    }
    let mut emitted_domains = HashMap::default();
    for domain in &mut resolver.domains {
        // Empty marker domains occupy the same semantic namespace as domains
        // that emit constants, so an explicit defpackage alias cannot duplicate
        // an unqualified marker unnoticed.
        if !domain.owner.contains('/') {
            domain.owner = format!("defpackage/{}", domain.owner);
        }
        if let Some(previous) = emitted_domains.insert(domain.owner.clone(), domain.origin.clone())
        {
            bail!(
                MappingDiagnostic::with_origins(
                    format!("duplicate semantic domain owner {}", domain.owner),
                    [previous, domain.origin.clone()]
                )
                .hint("choose distinct domain names; an omitted package means defpackage")
            );
        }
    }
    resolver.validate_domains()?;
    resolver.inherit_contracts()?;
    for domain in &resolver.domains {
        if domain.generated() {
            resolver
                .origins
                .reserved_classes
                .insert(domain.owner.clone(), domain.origin.clone());
            ensure!(
                program
                    .id(&domain.owner)
                    .is_none_or(|id| program.is_own(id))
                    && !resolver
                        .names
                        .classes
                        .values()
                        .any(|name| name == &domain.owner),
                MappingDiagnostic::with_origins(
                    format!(
                        "generated semantic domain collides with class {}",
                        domain.owner
                    ),
                    [domain.origin.clone()].into_iter().chain(
                        resolver
                            .names
                            .classes
                            .iter()
                            .filter(|(_, name)| *name == &domain.owner)
                            .filter_map(|(id, _)| resolver.origins.classes.get(id).cloned())
                    )
                )
                .hint("give the generated domain and mapped class distinct names")
            );
            resolver.names.reserved_classes.insert(domain.owner.clone());
        }
    }
    Ok(MappingPlan {
        names: resolver.names,
        origins: resolver.origins,
        diagnostics: resolver.diagnostics,
        domains: resolver
            .domains
            .into_iter()
            .map(PendingDomain::freeze)
            .collect(),
        bindings: resolver
            .bindings
            .into_iter()
            .map(|(target, binding)| (target, binding.value))
            .collect(),
        real_values: resolver
            .real_values
            .into_iter()
            .map(|(field, binding)| (field, binding.value))
            .collect(),
        calls: resolver
            .calls
            .into_iter()
            .map(|(site, binding)| (site, binding.value))
            .collect(),
        ignored_classes: resolver.ignored_classes,
    })
}

pub(crate) fn qualified(package: &str, name: &str) -> String {
    if package.is_empty() {
        name.into()
    } else {
        format!("{package}.{name}")
    }
}
pub(crate) fn has(annotations: &[Annotation], name: &str) -> bool {
    annotations.iter().any(|a| a.name == name)
}
pub(crate) fn domain_kind(annotations: &[Annotation]) -> Result<Option<DomainKind>> {
    let mut kinds = annotations.iter().filter_map(|a| match a.name.as_str() {
        "ValueDomain" => Some(DomainKind::Value),
        "FlagDomain" => Some(DomainKind::Flags),
        "SlotDomain" => Some(DomainKind::Slots),
        "PackedDomain" => Some(DomainKind::Packed),
        "NumericDomain" => Some(DomainKind::Numeric),
        "StringDomain" => Some(DomainKind::String),
        _ => None,
    });
    let kind = kinds.next();
    ensure!(
        kinds.next().is_none(),
        "a declaration may have only one domain kind"
    );
    Ok(kind)
}

impl Resolver<'_, '_> {
    pub(super) fn domain(&self, id: DomainId) -> &PendingDomain {
        &self.domains[id.0]
    }

    fn register(&mut self, source: &ParsedSource, declaration: &Declaration) -> Result<()> {
        ensure!(
            is_identifier(&declaration.name),
            "invalid mapped class name {}",
            declaration.name
        );
        let readable = qualified(&source.package, &declaration.name);
        if let Some(kind) = domain_kind(&declaration.annotations)? {
            ensure!(
                declaration.interface
                    && declaration.original.is_none()
                    && !has(&declaration.annotations, "External")
                    && !has(&declaration.annotations, "AlreadyMapped"),
                "semantic domains require an interface without a bytecode owner"
            );
            let id = DomainId(self.domains.len());
            if let Some(previous) = self.domain_names.get(&readable) {
                bail!(
                    MappingDiagnostic::with_origins(
                        format!("duplicate semantic domain {readable}"),
                        [
                            self.domains[previous.0].origin.clone(),
                            declaration.origin.clone()
                        ]
                    )
                    .hint("give each semantic domain a unique qualified name")
                );
            }
            self.domain_names.insert(readable.clone(), id);
            self.domains.push(PendingDomain {
                origin: declaration.origin.clone(),
                owner: readable.replace('.', "/"),
                kind,
                values: Vec::new(),
                exclusive_masks: Vec::new(),
                bit_fields: Vec::new(),
                format: None,
            });
            return Ok(());
        }
        for annotation in &declaration.annotations {
            ensure!(
                ["External", "AlreadyMapped"].contains(&annotation.name.as_str()),
                MappingDiagnostic::new(
                    format!("unsupported class annotation @{}", annotation.name),
                    annotation.origin.clone()
                )
            );
            annotation
                .allowed(&[])
                .map_err(|error| annotation.origin.attach(error))?;
        }
        let external = has(&declaration.annotations, "External");
        let ignored = has(&declaration.annotations, "AlreadyMapped");
        ensure!(!(external && ignored), "@External cannot be @AlreadyMapped");
        let original = if external || ignored {
            ensure!(
                declaration.original.is_none(),
                "@External/@AlreadyMapped must use their bytecode name without was"
            );
            readable.replace('.', "/")
        } else {
            let raw = declaration
                .original
                .as_deref()
                .unwrap_or(&readable)
                .replace('.', "/");
            if !raw.contains('/') && self.program.id(&raw).is_none() {
                let relative = qualified(&source.package, &raw).replace('.', "/");
                if self.program.id(&relative).is_some() {
                    relative
                } else {
                    raw
                }
            } else {
                raw
            }
        };
        let owner = self.program.id(&original).ok_or_else(|| {
            let mut issue = MappingDiagnostic::new(format!("class owner not found: {original}"), declaration.origin.clone())
                .hint("add /* was <original owner> */ when renaming; external owners must be present in the API classpath");
            let mut candidates: Vec<_> = self.program.classes().iter().map(|class| &class.name)
                .filter(|name| similar_name(&name.replace('/', "."), &original.replace('/', "."))).collect();
            candidates.sort();
            issue.notes.extend(candidates.into_iter().take(8).map(|name| format!("candidate: {name}")));
            anyhow::Error::from(issue)
        })?;
        ensure!(
            external != self.program.is_own(owner),
            "{} class {original} must be in the {}",
            if external { "@External" } else { "mapped" },
            if external {
                "API classpath"
            } else {
                "input program"
            }
        );
        if let Some(previous) = self.classes.insert(readable.clone(), owner) {
            ensure!(
                previous == owner,
                MappingDiagnostic::with_origins(
                    format!("duplicate mapped class target {readable}"),
                    self.origins
                        .classes
                        .get(&previous)
                        .cloned()
                        .into_iter()
                        .chain([declaration.origin.clone()])
                )
                .hint("choose distinct target names for different original classes")
            );
        }
        if ignored {
            ensure!(
                declaration.members.is_empty(),
                "@AlreadyMapped must have an empty body"
            );
            ensure!(
                !self.names.classes.contains_key(&owner),
                "class cannot be both @AlreadyMapped and renamed"
            );
            self.ignored_classes.insert(owner);
        } else if !external {
            ensure!(
                !self.ignored_classes.contains(&owner),
                "class cannot be both @AlreadyMapped and renamed"
            );
            let target = readable.replace('.', "/");
            insert_name(
                &mut self.names.classes,
                &mut self.origins.classes,
                owner,
                target,
                &declaration.origin,
            )?;
        }
        Ok(())
    }

    pub(crate) fn domain_id(&self, source: &ParsedSource, name: &str) -> Result<DomainId> {
        let candidates = self.resolve_name(source, name, |candidate| {
            self.domain_names.get(candidate).copied()
        })?;
        Ok(candidates)
    }

    fn resolve_name<T: Clone + PartialEq>(
        &self,
        source: &ParsedSource,
        name: &str,
        lookup: impl Fn(&str) -> Option<T>,
    ) -> Result<T> {
        let missing = |name: &str| {
            let mut candidates: Vec<_> = self
                .classes
                .keys()
                .chain(self.domain_names.keys())
                .cloned()
                .chain(self.known_types.iter().map(|name| name.replace('/', ".")))
                .filter(|candidate| lookup(candidate).is_some() && similar_name(candidate, name))
                .collect();
            candidates.sort();
            candidates.dedup();
            let mut issue = MappingDiagnostic::with_origins(format!("unknown type/domain {name}"), [])
                .hint("check the spelling and import; bytecode types must be present in the input or API classpath");
            issue.notes.extend(
                candidates
                    .into_iter()
                    .take(8)
                    .map(|name| format!("candidate: {name}")),
            );
            anyhow::Error::from(issue)
        };
        if name.contains('.') {
            return lookup(name).ok_or_else(|| missing(name));
        }
        if let Some(import) = source.explicit_import(name) {
            return lookup(import).ok_or_else(|| missing(import));
        }
        if let Some(value) = lookup(&qualified(&source.package, name)) {
            return Ok(value);
        }
        if let Some(value) = lookup(&format!("java.lang.{name}")) {
            return Ok(value);
        }
        let mut imports = Vec::new();
        for import in &source.imports {
            if let Some(package) = import.strip_suffix(".*")
                && let Some(value) = lookup(&qualified(package, name))
                && !imports.contains(&value)
            {
                imports.push(value);
            }
        }
        ensure!(imports.len() <= 1, "ambiguous imported name {name}");
        if let Some(value) = imports.first() {
            return Ok(value.clone());
        }
        if let Some(value) = lookup(name) {
            return Ok(value);
        }
        let mut matches = Vec::new();
        for candidate in self.classes.keys().chain(self.domain_names.keys()) {
            if candidate.rsplit('.').next() == Some(name)
                && let Some(value) = lookup(candidate)
                && !matches.contains(&value)
            {
                matches.push(value);
            }
        }
        if matches.is_empty() {
            return Err(missing(name));
        }
        ensure!(
            matches.len() == 1,
            "ambiguous type/domain {name}; use a qualified name or explicit import"
        );
        Ok(matches[0].clone())
    }

    pub(crate) fn ty(&self, source: &ParsedSource, ty: &Type, allow_void: bool) -> Result<JvmType> {
        (|| {
            let mut value = match ty.name.as_str() {
                "void" => {
                    ensure!(
                        allow_void && ty.dimensions == 0,
                        "void is only valid as a method return"
                    );
                    JvmType::Void
                }
                "boolean" => JvmType::Boolean,
                "byte" => JvmType::Byte,
                "short" => JvmType::Short,
                "char" => JvmType::Char,
                "int" => JvmType::Int,
                "long" => JvmType::Long,
                "float" => JvmType::Float,
                "double" => JvmType::Double,
                name => {
                    let owner = self.resolve_name(source, name, |candidate| {
                        self.classes
                            .get(candidate)
                            .or_else(|| {
                                candidate
                                    .strip_prefix("defpackage.")
                                    .and_then(|alias| self.classes.get(alias))
                            })
                            .map(|id| self.program.class(*id).name.clone())
                            .or_else(|| {
                                let internal = candidate.replace('.', "/");
                                self.known_types.get(&internal).cloned().or_else(|| {
                                    let pieces: Vec<_> = candidate.split('.').collect();
                                    (1..pieces.len()).rev().find_map(|i| {
                                        self.known_types
                                            .get(&format!(
                                                "{}${}",
                                                pieces[..i].join("/"),
                                                pieces[i..].join("$")
                                            ))
                                            .cloned()
                                    })
                                })
                            })
                    })?;
                    JvmType::Object(owner)
                }
            };
            for _ in 0..ty.dimensions {
                value = JvmType::Array(Box::new(value));
            }
            Ok(value)
        })()
        .map_err(|error| ty.origin.attach(error))
    }

    fn member(
        &mut self,
        source: &ParsedSource,
        declaration: &Declaration,
        owner: ClassId,
        member: &Member,
    ) -> Result<()> {
        let external = has(&declaration.annotations, "External");
        ensure!(
            member.name == "<init>" || is_identifier(&member.name),
            "invalid mapped member name {}",
            member.name
        );
        let original = if member.name == "<init>" {
            "<init>"
        } else if external {
            ensure!(
                member.original.is_none(),
                "@External members must not have was comments"
            );
            &member.name
        } else {
            member.original.as_deref().ok_or_else(|| {
                anyhow::anyhow!("missing /* was <original member> */ for {}", member.name)
            })?
        };
        let ty = match self.ty(source, &member.ty, member.parameters.is_some()) {
            Ok(ty) => ty,
            // A partial API can omit both an overload and the types appearing
            // only in that overload. Actual declaration descriptors populated
            // known_types above, so an unresolvable builtin type cannot match.
            Err(_) if source.builtin => return Ok(()),
            Err(error) => return Err(error),
        };
        if let Some(parameters) = &member.parameters {
            let parameters_ty = parameters
                .iter()
                .map(|p| self.ty(source, &p.ty, false))
                .collect::<Result<Vec<_>>>();
            let parameters_ty = match parameters_ty {
                Ok(types) => types,
                Err(_) if source.builtin => return Ok(()),
                Err(error) => return Err(error),
            };
            let descriptor = MethodDescriptor {
                parameters: parameters_ty,
                return_type: ty.clone(),
            };
            let descriptor_string = descriptor.descriptor();
            let id = self
                .program
                .declared_method(owner, original, &descriptor_string);
            if id.is_none() && source.builtin {
                return Ok(());
            }
            let id = id.ok_or_else(|| {
                self.missing_member(owner, original, &descriptor_string, true, &member.origin)
            })?;
            let mut seen = HashSet::default();
            for parameter in parameters {
                ensure!(
                    is_identifier(&parameter.name) && seen.insert(&parameter.name),
                    MappingDiagnostic::new(
                        format!("invalid or duplicate parameter name {}", parameter.name),
                        parameter.origin.clone()
                    )
                    .hint("use distinct Java identifiers for parameters of this method")
                );
            }
            if !external {
                insert_name(
                    &mut self.names.methods,
                    &mut self.origins.methods,
                    id,
                    member.name.clone(),
                    &member.origin,
                )?;
                self.origins
                    .parameters
                    .entry(id)
                    .or_insert_with(|| member.origin.clone());
                let names: Vec<_> = parameters.iter().map(|p| p.name.clone()).collect();
                if let Some(previous) = self.names.parameters.insert(id, names.clone()) {
                    ensure!(previous == names, "conflicting parameter names");
                }
            }
            self.bind(
                source,
                Target::Return(id),
                &ty,
                &member.annotations,
                Some(&descriptor),
            )?;
            for (index, parameter) in parameters.iter().enumerate() {
                self.bind(
                    source,
                    Target::Parameter { method: id, index },
                    &descriptor.parameters[index],
                    &parameter.annotations,
                    Some(&descriptor),
                )?;
            }
            for annotation in member.annotations.iter().filter(|a| a.name == "CallDomain") {
                self.call(source, id, annotation)
                    .map_err(|error| annotation.origin.attach(error))?;
            }
        } else {
            let descriptor = ty.descriptor();
            let id = self.program.declared_field(owner, original, &descriptor);
            if id.is_none() && source.builtin {
                return Ok(());
            }
            let id = id.ok_or_else(|| {
                self.missing_member(owner, original, &descriptor, false, &member.origin)
            })?;
            if !external {
                insert_name(
                    &mut self.names.fields,
                    &mut self.origins.fields,
                    id,
                    member.name.clone(),
                    &member.origin,
                )?;
            }
            for annotation in member
                .annotations
                .iter()
                .filter(|a| a.name == "DomainValue")
            {
                (|| -> Result<()> {
                    annotation.allowed(&["value"])?;
                    let domain = self.domain_id(source, annotation.class("value")?)?;
                    let field = self.program.field(id);
                    ensure!(
                        field.access_flags & 0x18 == 0x18,
                        "@DomainValue requires a static final field"
                    );
                    let constant = field
                        .constant_value
                        .ok_or_else(|| anyhow::anyhow!("@DomainValue requires ConstantValue"))?;
                    let pool = &self.program.class(owner).constant_pool;
                    let value = match pool.get(constant)? {
                        Constant::Integer(value) => ConstantValue::Integral(*value as i64),
                        Constant::Long(value) => ConstantValue::Integral(*value),
                        Constant::String(_) => ConstantValue::String(
                            String::from_utf16(&pool.string_utf16(constant)?).context(
                                "semantic String constant contains an unpaired UTF-16 surrogate",
                            )?,
                        ),
                        _ => bail!("unsupported @DomainValue constant"),
                    };
                    if insert_binding(
                        &mut self.real_values,
                        &mut self.diagnostics,
                        id,
                        Binding::new(source, &annotation.origin, domain),
                        BindingKind::Value,
                    )? {
                        self.domains[domain.0].values.push((
                            DomainValue {
                                name: member.name.clone(),
                                descriptor: descriptor.clone(),
                                value,
                                source: Some(id),
                                slot_domain: None,
                            },
                            annotation.origin.clone(),
                        ));
                    }
                    Ok(())
                })()
                .map_err(|error| annotation.origin.attach(error))?;
            }
            self.bind(source, Target::Field(id), &ty, &member.annotations, None)?;
        }
        Ok(())
    }

    fn missing_member(
        &self,
        owner: ClassId,
        name: &str,
        descriptor: &str,
        method: bool,
        origin: &MappingOrigin,
    ) -> anyhow::Error {
        let class = self.program.class(owner);
        let (kind, separator) = if method {
            ("method", "")
        } else {
            ("field", ":")
        };
        let mut issue = MappingDiagnostic::new(format!("{kind} not found: {}.{name}{separator}{descriptor}", class.name), origin.clone())
            .hint("match the original name and complete parameter/return type in the bytecode; update the declaration or its /* was ... */ comment");
        let mut candidates: Vec<(&str, &str)> = if method {
            class
                .methods
                .iter()
                .map(|member| (member.name.as_str(), member.descriptor.as_str()))
                .collect()
        } else {
            class
                .fields
                .iter()
                .map(|member| (member.name.as_str(), member.descriptor.as_str()))
                .collect()
        };
        let same_name = candidates.iter().any(|(candidate, _)| *candidate == name);
        candidates.retain(|(candidate, signature)| {
            if same_name {
                *candidate == name
            } else {
                *signature == descriptor
            }
        });
        if method {
            let wanted = MethodDescriptor::parse(descriptor).expect("resolved method descriptor");
            candidates.sort_by_key(|(name, descriptor)| {
                let parsed =
                    MethodDescriptor::parse(descriptor).expect("program method descriptor");
                (
                    parsed.parameters.len().abs_diff(wanted.parameters.len()),
                    parsed.return_type != wanted.return_type,
                    *name,
                    *descriptor,
                )
            });
        } else {
            candidates.sort_unstable();
        }
        for (candidate, signature) in candidates.iter().take(8) {
            issue.notes.push(format!(
                "candidate: {}.{candidate}{separator}{signature}",
                class.name
            ));
        }
        if candidates.len() > 8 {
            issue
                .notes
                .push(format!("{} more candidates", candidates.len() - 8));
        }
        issue.into()
    }

    fn call(
        &mut self,
        source: &ParsedSource,
        method: MemberId,
        annotation: &Annotation,
    ) -> Result<()> {
        annotation.allowed(&["value", "offset", "parameter"])?;
        let offset = u32::try_from(annotation.nonnegative("offset", None)?)
            .map_err(|_| anyhow::anyhow!("call offset out of range"))?;
        let parameter = annotation
            .args
            .get("parameter")
            .map(|_| annotation.nonnegative("parameter", None))
            .transpose()?;
        let domain = self.domain_id(source, annotation.class("value")?)?;
        let code = self
            .program
            .method(method)
            .code
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("@CallDomain requires method bytecode"))?;
        let instructions = crate::bytecode::decode(&code.bytes)?;
        let instruction = instructions
            .iter()
            .find(|i| i.pc == offset && matches!(i.opcode, 0xb6..=0xb9))
            .ok_or_else(|| {
                anyhow::anyhow!("@CallDomain offset {offset} is not an invocation instruction")
            })?;
        let reference = self
            .program
            .class(method.class)
            .constant_pool
            .member_ref(instruction.constant_index()?)?;
        let descriptor = MethodDescriptor::parse(reference.descriptor)?;
        let ty = match parameter {
            Some(index) => descriptor
                .parameters
                .get(index)
                .ok_or_else(|| anyhow::anyhow!("@CallDomain argument out of range"))?,
            None => &descriptor.return_type,
        };
        self.scalar_type(domain, ty)?;
        let site = CallSite {
            method,
            offset,
            parameter,
        };
        insert_binding(
            &mut self.calls,
            &mut self.diagnostics,
            site,
            Binding::new(source, &annotation.origin, domain),
            BindingKind::Call,
        )?;
        Ok(())
    }

    fn inherit_contracts(&mut self) -> Result<()> {
        // Resolve all explicit ancestors before inserting inherited facts. A nearest
        // explicit contract hides a farther one for this whole target, independently
        // for each return/argument; unrelated interface branches must agree.
        let explicit = self.bindings.clone();
        for owner in 0..self.program.classes().len() {
            for (index, method) in self.program.class(owner).methods.iter().enumerate() {
                if method.name.starts_with('<') || method.access_flags & 0xa != 0 {
                    continue;
                }
                let member = MemberId {
                    class: owner,
                    index,
                };
                let descriptor = MethodDescriptor::parse(&method.descriptor)?;
                for target in std::iter::once(Target::Return(member)).chain(
                    (0..descriptor.parameters.len()).map(|index| Target::Parameter {
                        method: member,
                        index,
                    }),
                ) {
                    if explicit.contains_key(&target) {
                        continue;
                    }
                    let candidates: Vec<_> = self
                        .program
                        .ancestors(owner)
                        .iter()
                        .filter_map(|ancestor| {
                            let id = self.program.declared_method(
                                *ancestor,
                                &method.name,
                                &method.descriptor,
                            )?;
                            let declaration = self.program.method(id);
                            if declaration.access_flags & 0xa != 0 {
                                return None;
                            }
                            if declaration.access_flags & 0x5 == 0
                                && package(&self.program.class(id.class).name)
                                    != package(&self.program.class(owner).name)
                            {
                                return None;
                            }
                            let parent = match target {
                                Target::Return(_) => Target::Return(id),
                                Target::Parameter { index, .. } => {
                                    Target::Parameter { method: id, index }
                                }
                                _ => unreachable!(),
                            };
                            explicit.get(&parent).map(|contract| (id, contract))
                        })
                        .collect();
                    let nearest: Vec<_> = candidates
                        .iter()
                        .filter(|(id, _)| {
                            !candidates.iter().any(|(other, _)| {
                                other.class != id.class
                                    && self.program.ancestors(other.class).contains(&id.class)
                            })
                        })
                        .collect();
                    if let Some((_, contract)) = nearest.first() {
                        if nearest
                            .iter()
                            .all(|(_, other)| other.value == contract.value)
                        {
                            self.bindings.insert(target, (*contract).clone());
                        } else if nearest.iter().any(|(_, binding)| !binding.builtin) {
                            self.diagnostics.push(MappingDiagnostic::with_origins(
                                format!("conflicting inherited contracts leave {}.{}{} {} unbound", self.program.class(owner).name, method.name, method.descriptor, match target { Target::Return(_) => "return".to_owned(), Target::Parameter { index, .. } => format!("parameter {index}"), _ => unreachable!() }),
                                nearest.iter().map(|(_, binding)| binding.origin.clone()),
                            ).warning().hint("add an explicit contract on this declaration, or make its inherited contracts agree"));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn similar_name(candidate: &str, wanted: &str) -> bool {
    let candidate = candidate.rsplit('.').next().unwrap();
    let wanted = wanted.rsplit('.').next().unwrap();
    let mut distances: Vec<usize> = (0..=wanted.chars().count()).collect();
    for (i, left) in candidate.chars().enumerate() {
        let mut diagonal = distances[0];
        distances[0] = i + 1;
        for (j, right) in wanted.chars().enumerate() {
            let previous = distances[j + 1];
            distances[j + 1] = (diagonal + usize::from(left != right))
                .min(previous + 1)
                .min(distances[j] + 1);
            diagonal = previous;
        }
    }
    distances.last().copied().unwrap_or(0) <= 2
}

fn insert_name<K: std::hash::Hash + Eq + Copy>(
    map: &mut HashMap<K, String>,
    origins: &mut HashMap<K, MappingOrigin>,
    key: K,
    value: String,
    origin: &MappingOrigin,
) -> Result<()> {
    if let Some(previous) = map.get(&key) {
        ensure!(
            previous == &value,
            MappingDiagnostic::with_origins(
                format!("conflicting explicit names {previous} and {value}"),
                origins
                    .get(&key)
                    .cloned()
                    .into_iter()
                    .chain([origin.clone()]),
            )
            .hint("use one mapped name for each original declaration")
        );
    }
    map.insert(key, value);
    origins.entry(key).or_insert_with(|| origin.clone());
    Ok(())
}
fn package(owner: &str) -> &str {
    owner.rsplit_once('/').map_or("", |(package, _)| package)
}
pub(crate) fn integral(ty: &JvmType) -> bool {
    matches!(
        ty,
        JvmType::Byte | JvmType::Short | JvmType::Char | JvmType::Int | JvmType::Long
    )
}
pub(crate) fn leaf_type(mut ty: &JvmType) -> &JvmType {
    while let JvmType::Array(inner) = ty {
        ty = inner;
    }
    ty
}
pub(super) fn collect_type(ty: &JvmType, names: &mut HashSet<String>) {
    if let JvmType::Object(name) = leaf_type(ty) {
        names.insert(name.clone());
    }
}
pub(crate) fn fits(ty: &JvmType, value: i64) -> bool {
    match ty {
        JvmType::Byte => i8::try_from(value).is_ok(),
        JvmType::Short => i16::try_from(value).is_ok(),
        JvmType::Char => u16::try_from(value).is_ok(),
        JvmType::Int => i32::try_from(value).is_ok(),
        JvmType::Long => true,
        _ => false,
    }
}
pub(crate) fn validate_cases(cases: &[ConditionalDomain]) -> Result<()> {
    let first = &cases[0];
    ensure!(
        cases.iter().all(|c| c.parameter == first.parameter),
        "@DomainWhen cases require one selector"
    );
    let equals: HashSet<_> = cases.iter().filter_map(|c| c.equals).collect();
    ensure!(
        equals.len() == cases.iter().filter(|c| c.equals.is_some()).count(),
        "duplicate @DomainWhen equality case"
    );
    let negatives: Vec<_> = cases.iter().filter_map(|c| c.not_equals).collect();
    let otherwise = cases.iter().filter(|c| c.otherwise).count();
    ensure!(
        negatives.len() <= 1 && otherwise <= 1,
        "overlapping @DomainWhen fallback cases"
    );
    if let Some(excluded) = negatives.first() {
        ensure!(
            otherwise == 0 && equals.iter().all(|value| value == excluded),
            "overlapping @DomainWhen notEquals cases"
        );
    }
    ensure!(
        otherwise == 0 || !equals.is_empty(),
        "@DomainWhen otherwise requires equality cases"
    );
    Ok(())
}

impl Annotation {
    pub(crate) fn allowed(&self, names: &[&str]) -> Result<()> {
        for name in self.args.keys() {
            ensure!(
                names.contains(&name.as_str()),
                MappingDiagnostic::new(
                    format!("unknown @{} argument {name}", self.name),
                    self.origin.clone()
                )
                .hint(if names.is_empty() {
                    "this annotation does not accept arguments".into()
                } else {
                    format!("accepted arguments: {}", names.join(", "))
                })
            );
        }
        Ok(())
    }
    pub(crate) fn class(&self, name: &str) -> Result<&str> {
        match self.args.get(name) {
            Some(Value::Class(value)) => Ok(value),
            _ => bail!("@{} {name} requires a class literal", self.name),
        }
    }
    pub(crate) fn integer(&self, name: &str, default: Option<i64>) -> Result<i64> {
        match self.args.get(name) {
            Some(Value::Integer(value)) => Ok(*value),
            None => default.ok_or_else(|| anyhow::anyhow!("@{} requires {name}", self.name)),
            _ => bail!("@{} {name} requires an integral literal", self.name),
        }
    }
    pub(crate) fn optional_integer(&self, name: &str) -> Result<Option<i64>> {
        self.args
            .get(name)
            .map(|_| self.integer(name, None))
            .transpose()
    }
    pub(crate) fn nonnegative(&self, name: &str, default: Option<usize>) -> Result<usize> {
        let value = self.integer(name, default.map(|x| x as i64))?;
        usize::try_from(value)
            .map_err(|_| anyhow::anyhow!("@{} {name} must be nonnegative", self.name))
    }
    pub(crate) fn boolean(&self, name: &str, default: bool) -> Result<bool> {
        match self.args.get(name) {
            Some(Value::Bool(value)) => Ok(*value),
            None => Ok(default),
            _ => bail!("@{} {name} requires a boolean literal", self.name),
        }
    }
    pub(crate) fn string(&self, name: &str) -> Result<&str> {
        match self.args.get(name) {
            Some(Value::String(value)) => Ok(value),
            _ => bail!("@{} {name} requires a string literal", self.name),
        }
    }
    pub(crate) fn optional_string(&self, name: &str) -> Result<Option<String>> {
        self.args
            .get(name)
            .map(|_| self.string(name).map(str::to_owned))
            .transpose()
    }
}
