//! Debug names describe exact JVM local lifetimes, never value dependencies.
//! Allocate one collision-free namespace before emitting headers and helper
//! bodies. Method-wide uniqueness is deliberately conservative: reconstructed
//! switches, elided try blocks, and constructor splits can widen Java scopes.
use super::types::TypeNames;
use crate::Result;
use crate::classfile::{Method, MethodDescriptor};
use crate::ir::MethodIr;
use crate::{
    naming::is_source_parameter,
    program::{MemberId, Program},
};
use std::collections::HashSet;

pub(super) fn parameters(
    method: &Method,
    member: MemberId,
    types: &TypeNames<'_>,
) -> Result<Vec<String>> {
    let descriptor = MethodDescriptor::parse(&method.descriptor)?;
    if let Some(authored) = types.names.parameter_names(member) {
        // Explicit bindings take precedence over advisory local reservations.
        // TypeNames has already selected qualifiers against these names, and
        // field accesses qualify shadowed members. Blank static final stores
        // still reserve their simple names in <clinit>, which has no parameters.
        return Ok(authored.to_vec());
    }
    let mut names = Allocator::new(types);
    let mut slot = u16::from(method.access_flags & 0x0008 == 0);
    Ok(descriptor
        .parameters
        .iter()
        .map(|ty| {
            let mut hints = method
                .code
                .iter()
                .flat_map(|code| &code.local_variables)
                .filter(|local| local.index == slot && local.start_pc == 0 && local.length != 0);
            let first = hints.next();
            let hint = first.filter(|first| {
                first.descriptor == ty.descriptor()
                    && usable_hint(&first.name)
                    && hints.all(|other| {
                        other.name == first.name
                            && other.descriptor == first.descriptor
                            && other.length == first.length
                    })
            });
            let fallback = format!("p{slot}");
            let name = names.allocate(hint.map_or(&fallback, |hint| &hint.name));
            slot += ty.slots();
            name
        })
        .collect())
}

pub(super) fn apply(ir: &mut MethodIr, parameters: &[String], types: &TypeNames<'_>) {
    let mut allocator = Allocator::new(types);
    allocator.used.extend(parameters.iter().cloned());
    // Synthesized carrier bindings are also named in generated declarations.
    // Keep those identities and reserve them against advisory debug names.
    for local in &ir.locals {
        if local.name.starts_with('$') {
            allocator.used.insert(local.name.clone());
        }
    }
    for local in &mut ir.locals {
        if let Some(index) = local.parameter
            && let Some(name) = parameters.get(index)
        {
            local.name.clone_from(name);
        }
    }
    let caught: HashSet<_> = ir
        .blocks
        .iter()
        .filter_map(|block| block.caught_exception)
        .filter_map(|value| ir.values[value].local)
        .collect();
    let mut active = super::local_flow::active_locals(ir);
    active.extend(&caught);
    let candidates: Vec<_> = ir
        .locals
        .iter_enumerated()
        .filter_map(|(index, local)| {
            (!local.is_this
                && local.parameter.is_none()
                && !local.name.starts_with('$')
                && active.contains(&index))
            .then_some(index)
        })
        .collect();
    // Meaningful hints take precedence over synthetic vN/eN spellings. A
    // fallback never steals the debug name of a later local in arena order.
    for &index in &candidates {
        if let Some(hint) = ir.locals[index]
            .name_hint
            .as_ref()
            .filter(|hint| usable_hint(&hint.name))
        {
            ir.locals[index].name = allocator.allocate(&hint.name);
        }
    }
    for index in candidates {
        if ir.locals[index]
            .name_hint
            .as_ref()
            .is_none_or(|hint| !usable_hint(&hint.name))
        {
            let stem = if caught.contains(&index) {
                "e"
            } else {
                &ir.locals[index].name
            };
            ir.locals[index].name = allocator.allocate(stem);
        }
    }
}

fn usable_hint(name: &str) -> bool {
    // The bundled Java 1.3 compiler reserves these inner-class support names
    // even in otherwise ordinary parameter/local declarations.
    is_source_parameter(name)
}

/// Semantic names are a source binding overlay. The frozen scheduled graph and
/// its local identities remain unchanged, and all declarations and reads share
/// this one allocation. Authored parameters and usable debug names take priority.
pub(super) fn semantic(
    ir: &MethodIr,
    program: &Program<'_>,
    types: &TypeNames<'_>,
    analysis: &crate::semantics::Analysis,
) -> Option<crate::index::IdVec<crate::ir::LocalId, String>> {
    let mut names: Option<crate::index::IdVec<crate::ir::LocalId, String>> = None;
    let mut allocator = None;
    for (id, local) in ir.locals.iter_enumerated() {
        if local.is_this
            || local.parameter.is_some()
            || local.name.starts_with('$')
            || local
                .name_hint
                .as_ref()
                .is_some_and(|hint| usable_hint(&hint.name))
        {
            continue;
        }
        if let Some(hint) = analysis.local_name(id).filter(|hint| usable_hint(hint))
            && hint != local.name
        {
            let names = names.get_or_insert_with(|| {
                ir.locals
                    .iter()
                    .map(|local| local.name.clone())
                    .collect::<Vec<_>>()
                    .into()
            });
            names[id] = allocator
                .get_or_insert_with(|| Allocator::generated(ir, program, types))
                .allocate(hint);
        }
    }
    names
}

pub(super) struct Allocator<'a> {
    reserved: &'a HashSet<String>,
    used: HashSet<String>,
}

impl<'a> Allocator<'a> {
    fn new(types: &'a TypeNames<'_>) -> Self {
        Self {
            reserved: types.local_reservations(),
            used: HashSet::new(),
        }
    }

    /// Renderer-only bindings share the source local namespace. Reserve own
    /// fields too, because their accesses may have been safely unqualified
    /// before this synthetic catch parameter enters scope.
    pub(super) fn generated(
        ir: &MethodIr,
        program: &Program<'_>,
        types: &'a TypeNames<'_>,
    ) -> Self {
        let mut allocator = Self::new(types);
        allocator
            .used
            .extend(ir.locals.iter().map(|local| local.name.clone()));
        for index in 0..program.class(types.owner).fields.len() {
            let field = MemberId {
                class: types.owner,
                index,
            };
            allocator
                .used
                .insert(types.names.field_name(field).to_owned());
        }
        allocator
    }

    pub(super) fn allocate(&mut self, stem: &str) -> String {
        let mut name = stem.to_owned();
        let mut suffix = 2;
        while self.reserved.contains(&name) || !self.used.insert(name.clone()) {
            name = format!("{stem}{suffix}");
            suffix += 1;
        }
        name
    }

    pub(super) fn reserve(&mut self, name: &str) {
        self.used.insert(name.to_owned());
    }
}
