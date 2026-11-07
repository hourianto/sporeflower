//! Program-wide checked-exception declarations and same-object rethrow facts.
//!
//! The JVM ignores optional Exceptions metadata. Source declarations therefore
//! need a fixed point over call sites, override families, and ordered handler
//! dispatch. Catch-local provenance is retained: catching an unchecked exception
//! as Throwable does not create a checked Throwable effect.
mod frames;
mod scan;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "../../tests/unit/throws_parallel.rs"]
mod parallel_tests;

use crate::index::{EntityId, IdVec};
use crate::program::{MemberId, Program};
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct ThrowFact {
    /// Reference type before Java local coalescing can erase it.
    pub source_type: String,
    /// Checked effects which actually reach this ATHROW, before its own catches.
    pub checked: Vec<String>,
    pub caught: bool,
    /// An unbounded effect forwarded through an implicit cleanup.
    /// It must never be cast to RuntimeException if a source rethrow remains.
    pub unknown: bool,
    /// Java 1.3 needs typed rethrow branches to express this narrowed effect set.
    /// Each branch must throw the original object, never a wrapper.
    pub requires_narrowing: bool,
}

#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub struct ThrowsPlan {
    methods: Vec<Vec<MethodEffects>>,
    calls: HashMap<String, HashMap<MemberId, CallEffects>>,
    reflection: HashMap<MemberId, BTreeMap<u32, CallEffects>>,
    boundaries: HashMap<MemberId, Vec<String>>,
    diagnostics: HashMap<MemberId, String>,
    throws: HashMap<MemberId, BTreeMap<u32, ThrowFact>>,
    instruction_throws: HashMap<MemberId, BTreeMap<u32, ThrowFact>>,
}

#[derive(Clone, Default)]
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub struct CallEffects {
    pub checked: Vec<String>,
    pub unknown: bool,
}

#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
struct MethodEffects {
    declared: Vec<String>,
    direct: CallEffects,
    runtime: CallEffects,
}

impl ThrowsPlan {
    pub fn declared(&self, member: MemberId) -> &[String] {
        &self.methods[member.class][member.index].declared
    }
    /// Runtime checked effects, including those hidden by a source callback boundary.
    pub fn escapes(&self, member: MemberId) -> &[String] {
        &self.methods[member.class][member.index].runtime.checked
    }
    pub fn unknown_escape(&self, member: MemberId) -> bool {
        self.methods[member.class][member.index].runtime.unknown
    }
    /// Constructor effects at this particular Class.newInstance instruction.
    /// PC uses normalized bytecode coordinates, like an IR value's PC.
    pub fn reflection_at(&self, member: MemberId, pc: u32) -> Option<&CallEffects> {
        self.reflection.get(&member)?.get(&pc)
    }
    /// Receiver-compatible effects for a bytecode call. Static/special calls use
    /// the exact declaration; virtual/interface calls use the symbolic owner.
    pub fn call_effects(
        &self,
        member: MemberId,
        owner: &str,
        virtual_dispatch: bool,
    ) -> &CallEffects {
        let method = &self.methods[member.class][member.index];
        if !virtual_dispatch {
            return &method.direct;
        }
        self.calls
            .get(owner)
            .and_then(|calls| calls.get(&member))
            .unwrap_or(&method.runtime)
    }
    /// Checked types requiring a same-object escape at a fixed external signature.
    pub fn boundary(&self, member: MemberId) -> &[String] {
        self.boundaries.get(&member).map_or(&[], Vec::as_slice)
    }
    pub fn diagnostic(&self, member: MemberId) -> Option<&str> {
        self.diagnostics.get(&member).map(String::as_str)
    }
    pub fn throw_type(&self, member: MemberId, pc: u32) -> Option<&str> {
        self.throw_fact(member, pc)
            .map(|fact| fact.source_type.as_str())
    }
    /// Diagnostic lookup in the original classfile, joining normalized JSR clones.
    pub fn throw_fact(&self, member: MemberId, pc: u32) -> Option<&ThrowFact> {
        self.throws.get(&member)?.get(&pc)
    }
    /// Lookup by the exact instruction origin, including normalized JSR clones.
    pub fn throw_at_instruction(
        &self,
        member: MemberId,
        instruction_pc: u32,
    ) -> Option<&ThrowFact> {
        self.instruction_throws.get(&member)?.get(&instruction_pc)
    }
    pub(crate) fn build(program: &Program<'_>) -> Self {
        Engine::new(program)
            .build(owned_methods(program).map(|member| scan_method(program, member)))
    }

    /// The caller prepares this plan before sharing the program with class
    /// workers. Parallel work must never run inside its lazy OnceLock initializer.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn build_parallel(program: &Program<'_>) -> Self {
        let methods: Vec<_> = owned_methods(program).collect();
        let summaries: Vec<_> = methods
            .into_par_iter()
            .map(|member| scan_method(program, member))
            .collect();
        // Indexed collection preserves class/method order. Only the independent
        // scans run in parallel; graph construction and propagation stay ordered.
        Engine::new(program).build(summaries)
    }
}

struct MethodScan {
    member: MemberId,
    body: Option<crate::Result<scan::Summary>>,
}

fn owned_methods(program: &Program<'_>) -> impl Iterator<Item = MemberId> {
    (0..program.own_count()).flat_map(|class| {
        (0..program.class(class).methods.len()).map(move |index| MemberId { class, index })
    })
}

fn scan_method(program: &Program<'_>, member: MemberId) -> MethodScan {
    let method = program.method(member);
    MethodScan {
        member,
        body: method
            .code
            .as_ref()
            .map(|_| scan::method(program, program.class(member.class), method)),
    }
}

pub fn is_checked(program: &Program<'_>, name: &str) -> bool {
    !program.is_subtype(name, "java/lang/RuntimeException")
        && !program.is_subtype(name, "java/lang/Error")
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct NodeId(usize);

impl EntityId for NodeId {
    fn from_index(index: usize) -> Self {
        Self(index)
    }
    fn index(self) -> usize {
        self.0
    }
}

struct Node {
    declared: BTreeSet<String>,
    hidden: BTreeSet<String>,
    unknown: bool,
    edges: Vec<Edge>,
    kind: NodeKind,
}

enum NodeKind {
    Method,
    Handler,
    /// Hidden and unknown effects stay implicit through cleanup handlers.
    Cleanup,
    Event(usize),
}

impl Node {
    fn new(kind: NodeKind) -> Self {
        Self {
            declared: BTreeSet::new(),
            hidden: BTreeSet::new(),
            unknown: false,
            edges: Vec::new(),
            kind,
        }
    }
}

struct MethodNode {
    node: NodeId,
    bounds: BTreeSet<MemberId>,
}

struct Edge {
    target: NodeId,
    limit: Option<String>,
    flow: Flow,
}
#[derive(Clone, Copy)]
enum Flow {
    All,
    Declared,
    Runtime,
    /// Reflection exposes constructor effects without a source throws clause.
    Reflective,
}
enum Effect {
    Declared(String),
    Hidden(String),
    Unknown,
}
#[derive(Clone, PartialEq, Eq, Hash)]
struct Handler {
    target: NodeId,
    catch_type: Option<String>,
}
#[derive(Clone, PartialEq, Eq, Hash)]
struct Scope {
    caller: MemberId,
    handlers: Vec<Handler>,
}
struct ThrowSite {
    node: NodeId,
    pc: u32,
    instruction_pc: u32,
    source_type: String,
    caught: bool,
}

struct Engine<'a, 'b> {
    program: &'a Program<'b>,
    methods: Vec<Vec<MethodNode>>,
    nodes: IdVec<NodeId, Node>,
    scopes: Vec<Scope>,
    scope_ids: HashMap<Scope, usize>,
    handler_nodes: HashMap<(MemberId, crate::cfg::BlockId), NodeId>,
    pending: VecDeque<(NodeId, Effect)>,
    diagnostics: HashMap<MemberId, BTreeSet<String>>,
    boundaries: HashMap<MemberId, BTreeSet<String>>,
    runtime_targets: HashMap<(String, MemberId), Vec<NodeId>>,
    own_descendants: Vec<Vec<usize>>,
    throws: HashMap<MemberId, Vec<ThrowSite>>,
    reflection: HashMap<MemberId, BTreeMap<u32, NodeId>>,
}

impl<'a, 'b> Engine<'a, 'b> {
    fn new(program: &'a Program<'b>) -> Self {
        let mut nodes = IdVec::new();
        let mut methods: Vec<Vec<_>> = program
            .classes()
            .iter()
            .map(|class| {
                class
                    .methods
                    .iter()
                    .map(|_| MethodNode {
                        node: nodes.push(Node::new(NodeKind::Method)),
                        bounds: BTreeSet::new(),
                    })
                    .collect()
            })
            .collect();
        let mut inherited = BTreeSet::new();
        // Throws clauses are directional contracts, unlike names. An override
        // may narrow its parent; an unrelated sibling's external no-throws
        // override must never constrain this method's declaration.
        for owner in 0..program.own_count() {
            let mut identities: HashMap<(&str, &str), Vec<MemberId>> = HashMap::new();
            for class in std::iter::once(owner).chain(program.ancestors(owner).iter().copied()) {
                for (index, method) in program.class(class).methods.iter().enumerate() {
                    if method.name.starts_with('<')
                        || method.access_flags & (0x0002 | 0x0008) != 0
                        || !program.visible_from(method.access_flags, class, owner)
                    {
                        continue;
                    }
                    let identity = (method.name.as_str(), method.descriptor.as_str());
                    identities
                        .entry(identity)
                        .or_default()
                        .push(MemberId { class, index });
                }
            }
            for ((name, descriptor), visible) in identities {
                let implementation = program
                    .declared_method(owner, name, descriptor)
                    .filter(|member| program.method(*member).access_flags & (0x0002 | 0x0008) == 0)
                    .or_else(|| {
                        let member =
                            program.resolve_method(&program.class(owner).name, name, descriptor)?;
                        // Inherited class methods can implement interfaces added by
                        // a subclass. Merely co-inheriting two interface declarations
                        // does not make either declaration an implementation.
                        (program.class(member.class).access_flags & 0x0200 == 0
                            && program.method(member).access_flags & (0x0002 | 0x0008) == 0)
                            .then_some(member)
                    });
                let Some(implementation) =
                    implementation.filter(|member| program.is_own(member.class))
                else {
                    continue;
                };
                let node = methods[implementation.class][implementation.index].node;
                for member in visible {
                    if member == implementation {
                        continue;
                    }
                    if program.is_own(member.class) {
                        inherited.insert((node, methods[member.class][member.index].node));
                    } else {
                        methods[implementation.class][implementation.index]
                            .bounds
                            .insert(member);
                    }
                }
            }
        }
        for (node, target) in inherited {
            nodes[node].edges.push(Edge {
                target,
                limit: None,
                flow: Flow::Declared,
            });
        }
        let mut own_descendants = vec![Vec::new(); program.classes().len()];
        for owner in 0..program.own_count() {
            own_descendants[owner].push(owner);
            for &parent in program.ancestors(owner) {
                own_descendants[parent].push(owner);
            }
        }
        Self {
            program,
            methods,
            nodes,
            scopes: Vec::new(),
            scope_ids: HashMap::new(),
            handler_nodes: HashMap::new(),
            pending: VecDeque::new(),
            diagnostics: HashMap::new(),
            boundaries: HashMap::new(),
            runtime_targets: HashMap::new(),
            own_descendants,
            throws: HashMap::new(),
            reflection: HashMap::new(),
        }
    }

    fn diagnostic(&mut self, member: MemberId, message: String) {
        self.diagnostics.entry(member).or_default().insert(message);
    }
    fn method_node(&self, member: MemberId) -> NodeId {
        self.methods[member.class][member.index].node
    }
    fn add(&mut self, node: NodeId, thrown: &str) {
        if is_checked(self.program, thrown) && self.nodes[node].declared.insert(thrown.to_owned()) {
            self.pending
                .push_back((node, Effect::Declared(thrown.to_owned())));
        }
    }
    fn add_hidden(&mut self, node: NodeId, thrown: &str) {
        if is_checked(self.program, thrown) && self.nodes[node].hidden.insert(thrown.to_owned()) {
            self.pending
                .push_back((node, Effect::Hidden(thrown.to_owned())));
        }
    }
    fn add_unknown(&mut self, node: NodeId) {
        if !std::mem::replace(&mut self.nodes[node].unknown, true) {
            self.pending.push_back((node, Effect::Unknown));
        }
    }
    fn escape(&mut self, member: MemberId, thrown: &str) {
        if !is_checked(self.program, thrown) {
            return;
        }
        if self.program.method(member).name == "<clinit>" {
            self.diagnostic(member, format!("static initializer lets checked exception {thrown} escape; Java has no throws clause for class initialization"));
            return;
        }
        let caller = self.method_node(member);
        for &bound in &self.methods[member.class][member.index].bounds {
            let method = self.program.method(bound);
            if !method
                .exceptions
                .iter()
                .any(|allowed| self.program.is_subtype(thrown, allowed))
            {
                let owner = &self.program.class(bound.class).name;
                if self.can_bridge(member, thrown) {
                    self.boundaries
                        .entry(member)
                        .or_default()
                        .insert(thrown.to_owned());
                    self.add_hidden(caller, thrown);
                } else {
                    self.diagnostic(member, format!("checked exception {thrown} cannot be declared without changing external contract {owner}.{}{}", method.name, method.descriptor));
                }
                return;
            }
        }
        self.add(caller, thrown);
    }
    fn can_bridge(&self, member: MemberId, thrown: &str) -> bool {
        self.program.method(member).code.is_some()
            && self.program.is_subtype(thrown, "java/lang/Throwable")
            && [
                "java/lang/InstantiationException",
                "java/lang/IllegalAccessException",
            ]
            .iter()
            .all(|reflection| {
                self.program.id(reflection).is_some()
                    && !self.program.is_subtype(thrown, reflection)
                    && !self.program.is_subtype(reflection, thrown)
            })
    }
    fn targets(&mut self, owner: &str, target: MemberId) -> Vec<NodeId> {
        let method = self.program.method(target);
        if method.access_flags & (0x0002 | 0x0008 | 0x0010) != 0 || method.name.starts_with('<') {
            return if self.program.is_own(target.class) {
                vec![self.method_node(target)]
            } else {
                Vec::new()
            };
        }
        let key = (owner.to_owned(), target);
        if let Some(targets) = self.runtime_targets.get(&key) {
            return targets.clone();
        }
        let mut targets = BTreeSet::new();
        if self.program.is_own(target.class) {
            targets.insert(self.method_node(target));
        }
        if let Some(owner) = self.program.id(owner) {
            for &class in &self.own_descendants[owner] {
                if let Some(target) = self.program.resolve_method(
                    &self.program.class(class).name,
                    &method.name,
                    &method.descriptor,
                ) && self.program.is_own(target.class)
                {
                    targets.insert(self.method_node(target));
                }
            }
        }
        let targets: Vec<_> = targets.into_iter().collect();
        self.runtime_targets.insert(key, targets.clone());
        targets
    }
    fn handler_node(&mut self, caller: MemberId, block: crate::cfg::BlockId) -> NodeId {
        if let Some(&node) = self.handler_nodes.get(&(caller, block)) {
            return node;
        }
        let node = self.nodes.push(Node::new(NodeKind::Handler));
        self.handler_nodes.insert((caller, block), node);
        node
    }
    fn event(&mut self, caller: MemberId, handlers: Vec<crate::cfg::HandlerEdge>) -> NodeId {
        let handlers = handlers
            .into_iter()
            .map(|handler| Handler {
                target: self.handler_node(caller, handler.target),
                catch_type: handler.catch_type,
            })
            .collect();
        let scope = Scope { caller, handlers };
        let scope_id = if let Some(&id) = self.scope_ids.get(&scope) {
            id
        } else {
            let id = self.scopes.len();
            self.scope_ids.insert(scope.clone(), id);
            self.scopes.push(scope);
            id
        };
        self.nodes.push(Node::new(NodeKind::Event(scope_id)))
    }
    fn narrowed(&self, thrown: &str, limit: Option<&str>) -> Option<String> {
        match limit {
            None => Some(thrown.to_owned()),
            Some(limit) if self.program.is_subtype(thrown, limit) => Some(thrown.to_owned()),
            Some(limit) if self.program.is_subtype(limit, thrown) => Some(limit.to_owned()),
            Some(_) => None,
        }
    }
    fn dispatch(&mut self, scope: usize, thrown: &str, hidden: bool) {
        let mut targets = Vec::new();
        let mut caught = false;
        for handler in &self.scopes[scope].handlers {
            match handler.catch_type.as_deref() {
                None => {
                    targets.push((handler.target, thrown.to_owned()));
                    caught = true;
                    break;
                }
                Some(catch) if self.program.is_subtype(thrown, catch) => {
                    targets.push((handler.target, thrown.to_owned()));
                    caught = true;
                    break;
                }
                Some(catch) if self.program.is_subtype(catch, thrown) => {
                    targets.push((handler.target, catch.to_owned()))
                }
                Some(_) => {}
            }
        }
        for (target, ty) in targets {
            if hidden && matches!(self.nodes[target].kind, NodeKind::Cleanup) {
                self.add_hidden(target, &ty);
            } else {
                self.add(target, &ty);
            }
        }
        if !caught {
            if hidden {
                self.add_hidden(self.method_node(self.scopes[scope].caller), thrown);
            } else {
                self.escape(self.scopes[scope].caller, thrown);
            }
        }
    }
    fn dispatch_unknown(&mut self, scope: usize) {
        let mut targets = Vec::new();
        let mut caught = false;
        for handler in &self.scopes[scope].handlers {
            let ty = handler
                .catch_type
                .as_deref()
                .unwrap_or("java/lang/Throwable");
            targets.push((handler.target, ty.to_owned()));
            if ty == "java/lang/Throwable" {
                caught = true;
                break;
            }
        }
        for (target, ty) in targets {
            if matches!(self.nodes[target].kind, NodeKind::Cleanup) {
                self.add_unknown(target);
            } else {
                self.add(target, &ty);
            }
        }
        if !caught {
            // Missing dependency metadata cannot justify narrowing a later
            // catchall rethrow. It does not itself invent a Java throws clause
            // on an ordinary invocation whose declaration is unavailable.
            self.add_unknown(self.method_node(self.scopes[scope].caller));
        }
    }

    fn runtime_effects(
        &self,
        member: MemberId,
        declared: &[String],
        targets: impl IntoIterator<Item = NodeId>,
    ) -> CallEffects {
        let mut types: BTreeSet<_> = declared.iter().cloned().collect();
        let mut unknown = constructor_reflection(self.program, member);
        for target in targets {
            types.extend(self.nodes[target].hidden.iter().cloned());
            unknown |= self.nodes[target].unknown;
        }
        CallEffects {
            checked: minimal(self.program, &types),
            unknown,
        }
    }

    fn build(mut self, summaries: impl IntoIterator<Item = MethodScan>) -> ThrowsPlan {
        for MethodScan { member, body } in summaries {
            let method = self.program.method(member);
            for thrown in &method.exceptions {
                self.escape(member, thrown);
            }
            let Some(body) = body else {
                continue;
            };
            let summary = match body {
                Ok(summary) => summary,
                Err(error) => {
                    self.diagnostic(
                        member,
                        format!("checked-exception analysis failed: {error:#}"),
                    );
                    continue;
                }
            };
            for block in summary.implicit_cleanups {
                let node = self.handler_node(member, block);
                self.nodes[node].kind = NodeKind::Cleanup;
            }
            for call in summary.calls {
                let event = self.event(member, call.handlers);
                let Some(target) = call.target else {
                    self.add_unknown(event);
                    continue;
                };
                if self.program.is_own(target.class) {
                    let target = self.method_node(target);
                    self.nodes[target].edges.push(Edge {
                        target: event,
                        limit: None,
                        flow: Flow::Declared,
                    });
                } else {
                    for thrown in &self.program.method(target).exceptions {
                        self.add(event, thrown);
                    }
                    // CLDC Class.newInstance propagates a nullary constructor's
                    // original exception, beyond its two declared reflection
                    // exceptions. Its native boundary cannot prove a rethrow
                    // limited to InstantiationException/IllegalAccessException.
                    if constructor_reflection(self.program, target) {
                        self.reflection
                            .entry(member)
                            .or_default()
                            .insert(call.pc, event);
                        if let Some(classes) = call.reflection_classes {
                            for name in classes {
                                let Some(class_id) = self.program.id(&name) else {
                                    self.add_unknown(event);
                                    continue;
                                };
                                let definition = self.program.class(class_id);
                                // Abstract/interface/array targets fail before invoking a
                                // constructor. Class access is checked by newInstance too.
                                if definition.access_flags & (0x0200 | 0x0400) != 0 {
                                    continue;
                                }
                                let Some(index) = definition.methods.iter().position(|method| {
                                    method.name == "<init>" && method.descriptor == "()V"
                                }) else {
                                    continue;
                                };
                                let constructor = MemberId {
                                    class: class_id,
                                    index,
                                };
                                if !self.program.visible_from(
                                    self.program.method(constructor).access_flags,
                                    class_id,
                                    member.class,
                                ) || !self.program.visible_from(
                                    definition.access_flags,
                                    class_id,
                                    member.class,
                                ) {
                                    continue;
                                }
                                if self.program.is_own(class_id) {
                                    let node = self.method_node(constructor);
                                    self.nodes[node].edges.push(Edge {
                                        target: event,
                                        limit: None,
                                        flow: Flow::Reflective,
                                    });
                                } else {
                                    for thrown in &self.program.method(constructor).exceptions {
                                        self.add_hidden(event, thrown);
                                    }
                                }
                            }
                        } else {
                            self.add_unknown(event);
                        }
                    }
                }
                let runtime_targets = if let Some(owner) = call.dispatch_owner {
                    self.targets(&owner, target)
                } else if self.program.is_own(target.class) {
                    vec![self.method_node(target)]
                } else {
                    Vec::new()
                };
                for target in runtime_targets {
                    self.nodes[target].edges.push(Edge {
                        target: event,
                        limit: None,
                        flow: Flow::Runtime,
                    });
                }
            }
            for site in summary.throws {
                let event = self.event(member, site.handlers);
                let caught = site
                    .value
                    .origins
                    .iter()
                    .any(|origin| matches!(origin, frames::Origin::Caught { .. }));
                if !caught {
                    // Ordinary source values need their static checked type;
                    // exception-origin narrowing is needed only for rethrows.
                    self.add(event, &site.value.source_type);
                } else {
                    for origin in site.value.origins {
                        match origin {
                            frames::Origin::Direct(ty) => self.add(
                                event,
                                if ty.as_ref() == "java/lang/Object" {
                                    "java/lang/Throwable"
                                } else {
                                    &ty
                                },
                            ),
                            frames::Origin::Caught { block, limit } => {
                                let handler = self.handler_node(member, block);
                                self.nodes[handler].edges.push(Edge {
                                    target: event,
                                    limit: limit.map(|ty| ty.to_string()),
                                    flow: Flow::All,
                                });
                            }
                        }
                    }
                }
                self.throws.entry(member).or_default().push(ThrowSite {
                    node: event,
                    pc: site.pc,
                    instruction_pc: site.instruction_pc,
                    source_type: site.value.source_type,
                    caught,
                });
            }
        }
        while let Some((node, effect)) = self.pending.pop_front() {
            if let Effect::Declared(thrown) = effect {
                let transfers: Vec<_> = self.nodes[node]
                    .edges
                    .iter()
                    .filter(|edge| !matches!(edge.flow, Flow::Runtime))
                    .filter_map(|edge| {
                        self.narrowed(&thrown, edge.limit.as_deref())
                            .map(|ty| (edge.target, ty, matches!(edge.flow, Flow::Reflective)))
                    })
                    .collect();
                for (target, ty, hidden) in transfers {
                    if hidden {
                        self.add_hidden(target, &ty);
                    } else {
                        self.add(target, &ty);
                    }
                }
                if let NodeKind::Event(scope) = self.nodes[node].kind {
                    self.dispatch(scope, &thrown, false);
                }
            } else if let Effect::Hidden(thrown) = effect {
                let transfers: Vec<_> = self.nodes[node]
                    .edges
                    .iter()
                    .filter(|edge| !matches!(edge.flow, Flow::Declared))
                    .filter_map(|edge| {
                        self.narrowed(&thrown, edge.limit.as_deref())
                            .map(|ty| (edge.target, ty))
                    })
                    .collect();
                for (target, ty) in transfers {
                    self.add_hidden(target, &ty);
                }
                if let NodeKind::Event(scope) = self.nodes[node].kind {
                    self.dispatch(scope, &thrown, true);
                }
            } else {
                let targets: Vec<_> = self.nodes[node]
                    .edges
                    .iter()
                    .filter(|edge| !matches!(edge.flow, Flow::Declared))
                    .map(|edge| (edge.target, edge.limit.clone()))
                    .collect();
                for (target, limit) in targets {
                    if let Some(limit) = limit {
                        self.add(target, &limit);
                    } else {
                        self.add_unknown(target);
                    }
                }
                if let NodeKind::Event(scope) = self.nodes[node].kind {
                    self.dispatch_unknown(scope);
                }
            }
        }
        let mut methods = Vec::new();
        for (class, definition) in self.program.classes().iter().enumerate() {
            let mut effects = Vec::new();
            for (index, method) in definition.methods.iter().enumerate() {
                let member = MemberId { class, index };
                let node = self.method_node(member);
                let declared = if self.program.is_own(class) {
                    let mut types = self.nodes[node].declared.clone();
                    types.extend(
                        method
                            .exceptions
                            .iter()
                            .filter(|name| !is_checked(self.program, name))
                            .cloned(),
                    );
                    minimal(self.program, &types)
                } else {
                    method.exceptions.clone()
                };
                let targets = self.targets(&definition.name, member);
                effects.push(MethodEffects {
                    direct: self.runtime_effects(member, &declared, [node]),
                    runtime: self.runtime_effects(member, &declared, targets),
                    declared,
                });
            }
            methods.push(effects);
        }
        let mut calls: HashMap<String, HashMap<MemberId, CallEffects>> = HashMap::new();
        for ((owner, member), targets) in &self.runtime_targets {
            let effects = self.runtime_effects(
                *member,
                &methods[member.class][member.index].declared,
                targets.iter().copied(),
            );
            calls
                .entry(owner.clone())
                .or_default()
                .insert(*member, effects);
        }
        let mut throws = HashMap::new();
        let mut instruction_throws = HashMap::new();
        for (member, sites) in self.throws {
            let mut facts = BTreeMap::<u32, ThrowFact>::new();
            let mut instructions = BTreeMap::new();
            for site in sites {
                let node = &self.nodes[site.node];
                let checked = minimal(self.program, &node.declared);
                let requires_narrowing = site.caught
                    && !node.unknown
                    && is_checked(self.program, &site.source_type)
                    && !checked
                        .iter()
                        .any(|ty| self.program.is_subtype(&site.source_type, ty));
                let fact = ThrowFact {
                    source_type: site.source_type,
                    checked,
                    caught: site.caught,
                    unknown: node.unknown,
                    requires_narrowing,
                };
                instructions.insert(site.instruction_pc, fact.clone());
                if let Some(previous) = facts.get_mut(&site.pc) {
                    let types = previous
                        .checked
                        .iter()
                        .chain(&fact.checked)
                        .cloned()
                        .collect();
                    previous.checked = minimal(self.program, &types);
                    if previous.source_type != fact.source_type {
                        previous.source_type = "java/lang/Throwable".into();
                    }
                    previous.caught |= fact.caught;
                    previous.unknown |= fact.unknown;
                    previous.requires_narrowing = previous.caught
                        && !previous.unknown
                        && is_checked(self.program, &previous.source_type)
                        && !previous
                            .checked
                            .iter()
                            .any(|ty| self.program.is_subtype(&previous.source_type, ty));
                } else {
                    facts.insert(site.pc, fact);
                }
            }
            throws.insert(member, facts);
            instruction_throws.insert(member, instructions);
        }
        ThrowsPlan {
            methods,
            calls,
            reflection: self
                .reflection
                .into_iter()
                .map(|(member, sites)| {
                    (
                        member,
                        sites
                            .into_iter()
                            .map(|(pc, node)| {
                                let node = &self.nodes[node];
                                let types = node.declared.union(&node.hidden).cloned().collect();
                                (
                                    pc,
                                    CallEffects {
                                        checked: minimal(self.program, &types),
                                        unknown: node.unknown,
                                    },
                                )
                            })
                            .collect(),
                    )
                })
                .collect(),
            boundaries: self
                .boundaries
                .into_iter()
                .map(|(member, types)| (member, minimal(self.program, &types)))
                .collect(),
            diagnostics: self
                .diagnostics
                .into_iter()
                .map(|(member, messages)| {
                    (member, messages.into_iter().collect::<Vec<_>>().join("; "))
                })
                .collect(),
            throws,
            instruction_throws,
        }
    }
}

fn minimal(program: &Program<'_>, types: &BTreeSet<String>) -> Vec<String> {
    types
        .iter()
        .filter(|name| {
            !types
                .iter()
                .any(|other| *name != other && program.is_subtype(name, other))
        })
        .cloned()
        .collect()
}

fn constructor_reflection(program: &Program<'_>, member: MemberId) -> bool {
    let method = program.method(member);
    program.class(member.class).name == "java/lang/Class"
        && method.name == "newInstance"
        && method.descriptor == "()Ljava/lang/Object;"
}
