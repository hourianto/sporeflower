//! Immutable original declarations, hierarchy indexes and shared analysis facts.
//! Source naming consumes this program without changing its JVM identities.
use crate::Result;
use crate::classfile::{ClassFile, Field, JvmType, Method, PRIVATE, PROTECTED, PUBLIC, package};
use anyhow::{bail, ensure};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock, RwLock};

mod invocations;
pub(crate) use invocations::InvocationTargets;

/// Whether supplied classes represent an application or a partial library.
/// Application scope assumes all application callers, writers and subclasses
/// were supplied. Platform callbacks and missing hierarchy metadata stay open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AnalysisScope {
    #[default]
    Partial,
    Application,
}

pub type ClassId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MemberId {
    pub class: ClassId,
    pub index: usize,
}

/// Whether an exact lookup can prove absence with the supplied declarations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemberResolution {
    Found(MemberId),
    Missing,
    Incomplete,
}

/// Borrowed input plus compact hierarchy indexes shared by all worker threads.
/// Own class IDs equal their indexes in the input slice. Duplicate library
/// entries are ignored; duplicate input declarations are an error.
pub struct Program<'a> {
    classes: Vec<&'a ClassFile>,
    pub(crate) input: Option<&'a crate::input::Input>,
    own_count: usize,
    by_name: HashMap<&'a str, ClassId>,
    ancestors: Vec<Vec<ClassId>>,
    descendants: Vec<Vec<ClassId>>,
    hierarchy_complete: Vec<bool>,
    invocation_targets: RwLock<HashMap<(ClassId, MemberId), Arc<InvocationTargets>>>,
    scope: AnalysisScope,
    methods: Vec<HashMap<(&'a str, &'a str), usize>>,
    fields: Vec<HashMap<(&'a str, &'a str), usize>>,
    normal_int_returns: Vec<Vec<OnceLock<Option<i32>>>>,
    boolean_producers: OnceLock<crate::ir::boolean_producers::Summary>,
    class_name_consumers: OnceLock<crate::semantics::class_names::Consumers>,
    throws_plan: OnceLock<crate::throws::ThrowsPlan>,
    initialization_plan: OnceLock<crate::initialization::InitializationPlan>,
}

impl<'a> Program<'a> {
    /// Analyze an immutable input, preserving access to its retained native bytes.
    pub fn from_input(input: &'a crate::input::Input, libraries: &'a [ClassFile]) -> Result<Self> {
        let mut program = Self::with_scope(input.classes(), libraries, input.analysis_scope())?;
        program.input = Some(input);
        Ok(program)
    }

    pub fn new(own: &'a [ClassFile], libraries: &'a [ClassFile]) -> Result<Self> {
        Self::with_scope(own, libraries, AnalysisScope::Partial)
    }

    pub fn with_scope(
        own: &'a [ClassFile],
        libraries: &'a [ClassFile],
        scope: AnalysisScope,
    ) -> Result<Self> {
        let mut classes = Vec::with_capacity(own.len() + libraries.len());
        let mut by_name = HashMap::with_capacity(own.len() + libraries.len());
        for class in own {
            ensure!(
                !by_name.contains_key(class.name.as_str()),
                "duplicate class {}",
                class.name
            );
            by_name.insert(class.name.as_str(), classes.len());
            classes.push(class);
        }
        for class in libraries {
            if !by_name.contains_key(class.name.as_str()) {
                by_name.insert(class.name.as_str(), classes.len());
                classes.push(class);
            }
        }
        let mut methods = Vec::with_capacity(classes.len());
        let mut fields = Vec::with_capacity(classes.len());
        for class in &classes {
            let mut method_index = HashMap::with_capacity(class.methods.len());
            for (index, method) in class.methods.iter().enumerate() {
                ensure!(
                    method_index
                        .insert((method.name.as_str(), method.descriptor.as_str()), index)
                        .is_none(),
                    "duplicate method {}.{}{}",
                    class.name,
                    method.name,
                    method.descriptor
                );
            }
            methods.push(method_index);
            let mut field_index = HashMap::with_capacity(class.fields.len());
            for (index, field) in class.fields.iter().enumerate() {
                ensure!(
                    field_index
                        .insert((field.name.as_str(), field.descriptor.as_str()), index)
                        .is_none(),
                    "duplicate field {}.{}:{}",
                    class.name,
                    field.name,
                    field.descriptor
                );
            }
            fields.push(field_index);
        }
        let mut program = Self {
            ancestors: vec![Vec::new(); classes.len()],
            descendants: vec![Vec::new(); classes.len()],
            hierarchy_complete: vec![false; classes.len()],
            invocation_targets: RwLock::new(HashMap::new()),
            scope,
            normal_int_returns: own
                .iter()
                .map(|class| class.methods.iter().map(|_| OnceLock::new()).collect())
                .collect(),
            class_name_consumers: OnceLock::new(),
            boolean_producers: OnceLock::new(),
            classes,
            input: None,
            own_count: own.len(),
            by_name,
            methods,
            fields,
            throws_plan: OnceLock::new(),
            initialization_plan: OnceLock::new(),
        };
        // Class graphs are small relative to method bodies. Memoized ancestor
        // lists keep naming linear in visible declarations, not all method pairs.
        let mut marks = vec![0u8; program.classes.len()];
        for id in 0..program.classes.len() {
            program.collect_ancestors(id, &mut marks)?;
        }
        for id in 0..program.classes.len() {
            program.descendants[id].push(id);
            for &ancestor in &program.ancestors[id] {
                program.descendants[ancestor].push(id);
            }
        }
        Ok(program)
    }

    fn collect_ancestors(&mut self, id: ClassId, marks: &mut [u8]) -> Result<()> {
        if marks[id] == 2 {
            return Ok(());
        }
        if marks[id] == 1 {
            bail!("cyclic class hierarchy at {}", self.classes[id].name);
        }
        marks[id] = 1;
        let class = self.classes[id];
        let parents: Vec<_> = class
            .super_name
            .iter()
            .chain(class.interfaces.iter())
            .filter_map(|name| self.id(name))
            .collect();
        let mut ancestors = Vec::new();
        let mut seen = HashSet::new();
        for parent in parents {
            self.collect_ancestors(parent, marks)?;
            if seen.insert(parent) {
                ancestors.push(parent);
            }
            for &ancestor in &self.ancestors[parent] {
                if seen.insert(ancestor) {
                    ancestors.push(ancestor);
                }
            }
        }
        self.ancestors[id] = ancestors;
        self.hierarchy_complete[id] =
            class
                .super_name
                .iter()
                .chain(&class.interfaces)
                .all(|parent| {
                    self.id(parent)
                        .map_or(parent == "java/lang/Object", |parent| {
                            self.hierarchy_complete[parent]
                        })
                });
        marks[id] = 2;
        Ok(())
    }

    pub fn class(&self, id: ClassId) -> &'a ClassFile {
        self.classes[id]
    }
    pub fn id(&self, name: &str) -> Option<ClassId> {
        self.by_name.get(name).copied()
    }
    pub fn classes(&self) -> &[&'a ClassFile] {
        &self.classes
    }
    pub fn own_count(&self) -> usize {
        self.own_count
    }
    pub fn is_own(&self, id: ClassId) -> bool {
        id < self.own_count
    }
    pub fn ancestors(&self, id: ClassId) -> &[ClassId] {
        &self.ancestors[id]
    }
    pub(crate) fn descendants(&self, id: ClassId) -> &[ClassId] {
        &self.descendants[id]
    }
    pub(crate) fn application_scope(&self) -> bool {
        self.scope == AnalysisScope::Application
    }
    pub(crate) fn complete_hierarchy(&self, class: ClassId) -> bool {
        self.hierarchy_complete[class]
    }
    pub fn method(&self, member: MemberId) -> &'a Method {
        &self.classes[member.class].methods[member.index]
    }
    /// Schedule a method using the program's immutable analysis facts.
    /// Initialization discovery and native remapping pass no source triggers;
    /// Java lowering supplies the sites from the completed initialization plan.
    pub(crate) fn decompile_method(
        &self,
        member: MemberId,
        initialization_sites: &[crate::ir::InitializationSite],
    ) -> Result<crate::ir::MethodIr> {
        crate::ir::decompile_method_with_context(
            self.class(member.class),
            self.method(member),
            &crate::ir::MethodContext {
                initialization_sites,
                normal_int_return: Some(&|method| self.normal_int_return(method)),
                reference_assignable: Some(&|from, to| self.assignable(from, to)),
            },
        )
    }
    /// A normal-result fact for an exact static application call. This says
    /// nothing about purity, termination, initialization or exceptional effects.
    pub fn normal_int_return(&self, method: &crate::ir::Member) -> Option<i32> {
        let target = self.resolve_method(&method.owner, &method.name, &method.descriptor)?;
        if !self.is_own(target.class) || self.method(target).access_flags & 0x0008 == 0 {
            return None;
        }
        *self.normal_int_returns[target.class][target.index].get_or_init(|| {
            crate::normal_returns::constant_int(self.class(target.class), self.method(target))
        })
    }
    /// Original return producers cover exact calls and the supplied application's
    /// virtual implementations. External declarations alone do not prove bits.
    pub(crate) fn canonical_boolean_return(
        &self,
        caller: Option<ClassId>,
        kind: crate::ir::InvokeKind,
        method: &crate::ir::Member,
    ) -> bool {
        self.boolean_producers()
            .canonical_return(self, caller, kind, method)
    }
    /// A certificate for this exact platform body, independent of overrides.
    /// Virtual families must also require every supplied overriding body.
    pub(crate) fn canonical_boolean_api_implementation(&self, target: MemberId) -> bool {
        !self.is_own(target.class)
            && crate::api::ApiCatalog::bundled().is_ok_and(|api| {
                let method = self.method(target);
                api.canonical_boolean_return(&crate::ir::Member {
                    owner: self.class(target.class).name.clone(),
                    name: method.name.clone(),
                    descriptor: method.descriptor.clone(),
                })
            })
    }
    pub(crate) fn exact_method(
        &self,
        caller: Option<ClassId>,
        kind: crate::ir::InvokeKind,
        method: &crate::ir::Member,
    ) -> Option<MemberId> {
        let mut target = self.resolve_method(&method.owner, &method.name, &method.descriptor)?;
        let flags = self.method(target).access_flags;
        let exact = match kind {
            crate::ir::InvokeKind::Static => flags & 0x0008 != 0,
            crate::ir::InvokeKind::Special => {
                if flags & 0x0008 != 0 {
                    return None;
                }
                if flags & (PRIVATE | 0x0010) == 0 && !method.name.starts_with('<') {
                    let caller = caller?;
                    let owner = self.id(&method.owner)?;
                    // ACC_SUPER selects from the caller's direct superclass,
                    // even when the constant pool names a more distant base.
                    if self.class(caller).access_flags & 0x0020 != 0
                        && owner != caller
                        && self.class(owner).access_flags & 0x0200 == 0
                    {
                        if !self.ancestors(caller).contains(&owner) {
                            return None;
                        }
                        target = self.resolve_method(
                            self.class(caller).super_name.as_deref()?,
                            &method.name,
                            &method.descriptor,
                        )?;
                    }
                }
                self.method(target).access_flags & 0x0008 == 0
            }
            crate::ir::InvokeKind::Virtual | crate::ir::InvokeKind::Interface => {
                flags & (PRIVATE | 0x0010) != 0
                    || self.class(target.class).access_flags & 0x0010 != 0
            }
        };
        exact.then_some(target)
    }
    /// Canonical writes observed in the supplied application, including
    /// feedback through other fields and application call arguments.
    pub(crate) fn canonical_boolean_field(&self, field: &crate::ir::Member) -> bool {
        field.descriptor == "Z"
            && self
                .resolve_field(&field.owner, &field.name, &field.descriptor)
                .is_some_and(|field| {
                    self.is_own(field.class) && self.boolean_producers().field(field)
                })
    }
    pub(crate) fn canonical_boolean_parameter(&self, method: MemberId, index: usize) -> bool {
        self.is_own(method.class) && self.boolean_producers().parameter(method, index)
    }
    /// A supplied application implementation may also receive API callbacks.
    /// Include interfaces adopted by descendants which inherit the body, and
    /// keep missing hierarchy metadata open rather than guessing its contract.
    pub(crate) fn external_method_inputs(&self, member: MemberId) -> bool {
        let method = self.method(member);
        if method.access_flags & PRIVATE != 0 {
            return false;
        }
        if !self.application_scope() {
            return true;
        }
        if method.access_flags & 0x0008 != 0 || method.name.starts_with('<') {
            return false;
        }
        self.descendants(member.class).iter().copied().any(|class| {
            if class != member.class && !self.ancestors(class).contains(&member.class)
                || self.resolve_method(&self.class(class).name, &method.name, &method.descriptor)
                    != Some(member)
            {
                return false;
            }
            if !self.is_own(class) {
                return true;
            }
            std::iter::once(class)
                .chain(self.ancestors(class).iter().copied())
                .any(|ancestor| {
                    let declaration = self.class(ancestor);
                    let external = !self.is_own(ancestor)
                        && self
                            .declared_method(ancestor, &method.name, &method.descriptor)
                            .is_some_and(|contract| {
                                let flags = self.method(contract).access_flags;
                                flags & (PRIVATE | 0x0008) == 0
                                    && self.visible_from(flags, ancestor, class)
                            });
                    external
                        || declaration
                            .super_name
                            .iter()
                            .chain(&declaration.interfaces)
                            .any(|parent| {
                                self.id(parent).is_none()
                                    && (parent != "java/lang/Object"
                                        || implicit_object_member(&method.name, &method.descriptor))
                            })
                })
        })
    }
    fn boolean_producers(&self) -> &crate::ir::boolean_producers::Summary {
        self.boolean_producers
            .get_or_init(|| crate::ir::boolean_producers::Summary::build(self))
    }
    pub(crate) fn class_name_consumers(&self) -> &crate::semantics::class_names::Consumers {
        self.class_name_consumers
            .get_or_init(|| crate::semantics::class_names::Consumers::build(self))
    }
    pub fn throws_plan(&self) -> &crate::throws::ThrowsPlan {
        self.throws_plan
            .get_or_init(|| crate::throws::ThrowsPlan::build(self))
    }

    /// Prepare checked-exception facts with the caller's worker pool, before
    /// sharing this program with class workers. Exclusive access keeps this
    /// phase separate from lazy callers, which retain a serial initializer.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn prepare_throws(&mut self, pool: &rayon::ThreadPool) {
        if self.throws_plan.get().is_none() {
            // No OnceLock initializer is held while Rayon can steal work. In
            // particular, a worker must not steal a class job that waits for
            // the same plan whose initialization it would then be suspending.
            let plan = pool.install(|| {
                if pool.current_num_threads() == 1 {
                    crate::throws::ThrowsPlan::build(self)
                } else {
                    crate::throws::ThrowsPlan::build_parallel(self)
                }
            });
            let _ = self.throws_plan.set(plan);
        }
    }

    /// Complete expensive shared summaries before publishing this program to
    /// class workers. Native callers can install their own Rayon pool; browser
    /// sessions use the same method summaries and solve serially.
    pub fn prepare_analysis(&mut self) {
        if self.boolean_producers.get().is_none() {
            #[cfg(not(target_arch = "wasm32"))]
            let summary = crate::ir::boolean_producers::Summary::build_parallel(self);
            #[cfg(target_arch = "wasm32")]
            let summary = crate::ir::boolean_producers::Summary::build(self);
            let _ = self.boolean_producers.set(summary);
        }
        if self.throws_plan.get().is_none() {
            #[cfg(not(target_arch = "wasm32"))]
            let plan = crate::throws::ThrowsPlan::build_parallel(self);
            #[cfg(target_arch = "wasm32")]
            let plan = crate::throws::ThrowsPlan::build(self);
            let _ = self.throws_plan.set(plan);
        }
    }

    pub fn initialization_plan(&self) -> &crate::initialization::InitializationPlan {
        self.initialization_plan
            .get_or_init(|| crate::initialization::InitializationPlan::build(self))
    }
    pub fn method_throws(&self, member: MemberId) -> &[String] {
        self.throws_plan().declared(member)
    }
    pub fn method_escapes(&self, member: MemberId) -> &[String] {
        self.throws_plan().escapes(member)
    }
    pub fn field(&self, member: MemberId) -> &'a Field {
        &self.classes[member.class].fields[member.index]
    }

    pub fn declared_method(
        &self,
        owner: ClassId,
        name: &str,
        descriptor: &str,
    ) -> Option<MemberId> {
        self.methods[owner]
            .get(&(name, descriptor))
            .copied()
            .map(|index| MemberId {
                class: owner,
                index,
            })
    }

    pub fn declared_field(&self, owner: ClassId, name: &str, descriptor: &str) -> Option<MemberId> {
        self.fields[owner]
            .get(&(name, descriptor))
            .copied()
            .map(|index| MemberId {
                class: owner,
                index,
            })
    }

    /// JVM class-method lookup: class/superclass declarations win over interfaces.
    pub fn resolve_method(&self, owner: &str, name: &str, descriptor: &str) -> Option<MemberId> {
        match self.method_resolution(owner, name, descriptor) {
            MemberResolution::Found(found) => Some(found),
            MemberResolution::Missing | MemberResolution::Incomplete => None,
        }
    }

    /// Missing dependency metadata is not a JVM linkage failure. Keep it distinct
    /// from a complete lookup which proves that the exact member does not exist.
    pub(crate) fn method_resolution(
        &self,
        owner: &str,
        name: &str,
        descriptor: &str,
    ) -> MemberResolution {
        let mut current = Some(owner);
        let mut interfaces = Vec::new();
        let mut seen = HashSet::new();
        while let Some(class_name) = current {
            // Direct declarations need no visited-set allocation.
            let id = self.id(class_name);
            if let Some(found) = id.and_then(|id| self.declared_method(id, name, descriptor)) {
                return MemberResolution::Found(found);
            }
            if !seen.insert(class_name) {
                return MemberResolution::Incomplete;
            }
            let Some(id) = id else {
                // Object is the Java language root even under --no-api. Only
                // its standard inherited members can leave lookup unresolved.
                if class_name == "java/lang/Object" && !implicit_object_member(name, descriptor) {
                    break;
                }
                return MemberResolution::Incomplete;
            };
            if name.starts_with('<') {
                return MemberResolution::Missing;
            }
            let class = self.class(id);
            interfaces.extend(class.interfaces.iter().map(String::as_str));
            current = class.super_name.as_deref();
        }
        let mut incomplete = false;
        while let Some(interface) = interfaces.pop() {
            if !seen.insert(interface) {
                continue;
            }
            let Some(id) = self.id(interface) else {
                incomplete = true;
                continue;
            };
            if let Some(found) = self.declared_method(id, name, descriptor) {
                return MemberResolution::Found(found);
            }
            interfaces.extend(self.class(id).interfaces.iter().map(String::as_str));
        }
        if incomplete {
            MemberResolution::Incomplete
        } else {
            MemberResolution::Missing
        }
    }

    pub(crate) fn field_resolution(
        &self,
        owner: &str,
        name: &str,
        descriptor: &str,
    ) -> MemberResolution {
        fn lookup(
            program: &Program<'_>,
            owner: &str,
            name: &str,
            descriptor: &str,
            seen: &mut HashSet<ClassId>,
        ) -> MemberResolution {
            let Some(id) = program.id(owner) else {
                return if owner == "java/lang/Object" {
                    MemberResolution::Missing
                } else {
                    MemberResolution::Incomplete
                };
            };
            if let Some(found) = program.declared_field(id, name, descriptor) {
                return MemberResolution::Found(found);
            }
            if !seen.insert(id) {
                return MemberResolution::Missing;
            }
            let class = program.class(id);
            for parent in class.interfaces.iter().chain(class.super_name.iter()) {
                match lookup(program, parent, name, descriptor, seen) {
                    MemberResolution::Missing => (),
                    result => return result,
                }
            }
            MemberResolution::Missing
        }
        lookup(self, owner, name, descriptor, &mut HashSet::new())
    }

    /// Field lookup order differs from methods: interfaces precede superclass.
    pub fn resolve_field(&self, owner: &str, name: &str, descriptor: &str) -> Option<MemberId> {
        match self.field_resolution(owner, name, descriptor) {
            MemberResolution::Found(found) => Some(found),
            MemberResolution::Missing | MemberResolution::Incomplete => None,
        }
    }

    pub fn is_subtype(&self, child: &str, parent: &str) -> bool {
        if child == parent || parent == "java/lang/Object" {
            return true;
        }
        let (Some(child), Some(parent)) = (self.id(child), self.id(parent)) else {
            return false;
        };
        self.ancestors[child].contains(&parent)
    }

    pub fn assignable(&self, from: &JvmType, to: &JvmType) -> bool {
        if from == to {
            return true;
        }
        match (from, to) {
            (JvmType::Object(child), JvmType::Object(parent)) => self.is_subtype(child, parent),
            (JvmType::Array(_), JvmType::Object(parent)) => matches!(
                parent.as_str(),
                "java/lang/Object" | "java/lang/Cloneable" | "java/io/Serializable"
            ),
            (JvmType::Array(child), JvmType::Array(parent)) => match (&**child, &**parent) {
                (
                    JvmType::Object(_) | JvmType::Array(_),
                    JvmType::Object(_) | JvmType::Array(_),
                ) => self.assignable(child, parent),
                _ => child == parent,
            },
            _ => false,
        }
    }

    /// JVM final stores are limited to the declaring class. Since classfile
    /// version 53, they are also limited to its matching initializer method.
    pub(crate) fn field_write_permitted(&self, field: MemberId, writer: MemberId) -> bool {
        let declaration = self.field(field);
        declaration.access_flags & 0x0010 == 0
            || field.class == writer.class
                && (self.class(writer.class).major_version < 53
                    || self.method(writer).name
                        == if declaration.access_flags & 0x0008 != 0 {
                            "<clinit>"
                        } else {
                            "<init>"
                        })
    }

    /// JVM member access independent of receiver constraints. Unlike inherited
    /// contract visibility, protected access requires the caller to be a subclass.
    pub(crate) fn member_accessible_from(
        &self,
        access: u16,
        declaring: ClassId,
        from: ClassId,
    ) -> bool {
        if declaring == from {
            return true;
        }
        if access & PRIVATE != 0 {
            return false;
        }
        if package(&self.class(declaring).name) == package(&self.class(from).name) {
            return true;
        }
        access & PUBLIC != 0
            || access & PROTECTED != 0
                && self.is_subtype(&self.class(from).name, &self.class(declaring).name)
    }

    pub fn visible_from(&self, access: u16, declaring: ClassId, from: ClassId) -> bool {
        if declaring == from {
            return true;
        }
        if access & PRIVATE != 0 {
            return false;
        }
        access & (PUBLIC | PROTECTED) != 0
            || package(&self.class(declaring).name) == package(&self.class(from).name)
    }
}

fn implicit_object_member(name: &str, descriptor: &str) -> bool {
    matches!(
        (name, descriptor),
        ("getClass", "()Ljava/lang/Class;")
            | ("hashCode", "()I")
            | ("equals", "(Ljava/lang/Object;)Z")
            | ("clone", "()Ljava/lang/Object;")
            | ("toString", "()Ljava/lang/String;")
            | ("notify" | "notifyAll" | "finalize", "()V")
            | ("wait", "()V" | "(J)V" | "(JI)V")
    )
}
