//! Semantic meanings are presentation facts over scheduled expression occurrences.
//! They never change JVM storage or the operations which produce a value.
pub mod class_names;
mod flow;
mod infer;
mod packed;
mod ranges;

use crate::index::IdVec;
use crate::ir::{BlockId, LocalId, MethodIr, ValueId};
use crate::mappings::{ContainerContract, Contract, DomainId, MappingPlan};
use crate::program::{MemberId, Program};
use std::collections::{BTreeMap, HashMap, HashSet};

/// An exact scheduled statement; the statement count denotes its terminator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Site {
    pub block: BlockId,
    pub statement: usize,
}

/// Alternative producer meanings. Empty and known is neutral evidence (a literal),
/// while `unknown` records a real opaque alternative and survives joins.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub domains: HashSet<DomainId>,
    pub arrays: HashSet<Contract>,
    pub containers: HashSet<ContainerContract>,
    pub unknown: bool,
    pub class_name: bool,
    dependent: HashSet<infer::Dependent>,
}

impl Facts {
    pub fn domain(&self) -> Option<DomainId> {
        (!self.unknown && self.dependent.is_empty() && self.domains.len() == 1)
            .then(|| *self.domains.iter().next().unwrap())
    }
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
    pub(super) fn unknown() -> Self {
        Self {
            unknown: true,
            ..Self::default()
        }
    }
    pub(super) fn scalar(domain: DomainId) -> Self {
        Self {
            domains: HashSet::from([domain]),
            ..Self::default()
        }
    }
    pub(super) fn merge(&mut self, other: &Self) {
        self.domains.extend(&other.domains);
        self.arrays.extend(other.arrays.iter().cloned());
        self.containers.extend(other.containers.iter().cloned());
        self.dependent.extend(other.dependent.iter().cloned());
        self.unknown |= other.unknown;
        self.class_name |= other.class_name;
    }
    pub(super) fn joined(mut self, other: &Self) -> Self {
        self.merge(other);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PackedPart {
    Mask,
    ClearMask,
    ValueMask,
    Shift,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PackedConstant {
    pub domain: DomainId,
    pub field: usize,
    pub part: PackedPart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OffsetConstant {
    pub domain: DomainId,
    pub slot: i64,
    pub multiplier: i64,
    pub addend: i64,
}

/// Original bytecode destination of a class-name input. Non-scalar boundaries
/// still participate in origin connectivity, but cannot use scalar adapters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClassNameDestination {
    Argument { offset: u32, parameter: usize },
    Field { offset: u32 },
    ArrayArgument { offset: u32, parameter: usize },
    Return,
    ArrayStore,
    NonScalarField,
}

#[derive(Clone, Debug)]
pub struct ClassNameInput {
    pub destination: ClassNameDestination,
    pub names: Vec<String>,
    pub origins: Vec<u32>,
    /// False when an opaque alternative may already contain mapped names.
    pub complete: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClassNameDecision {
    RewriteProducers,
    AdaptConsumers,
    Reject,
}

/// One connected component of original producers and all their consumers.
/// A component changes provenance as a unit: original text enters an adapter;
/// relocated producer text never does. Native width constraints can promote the
/// same component to adaptation without recomputing transitive closure.
#[derive(Clone, Debug)]
pub struct ClassNameGroup {
    pub origins: Vec<u32>,
    pub inputs: Vec<ClassNameInput>,
    pub decision: ClassNameDecision,
}

impl ClassNameGroup {
    pub fn require_adapter(&mut self) {
        self.decision = if self.inputs.iter().all(|input| {
            input.complete
                && matches!(
                    input.destination,
                    ClassNameDestination::Argument { .. } | ClassNameDestination::Field { .. }
                )
        }) {
            ClassNameDecision::AdaptConsumers
        } else {
            ClassNameDecision::Reject
        };
    }
}

/// Borrowed Java-facing projections of the connected adaptation plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassNameArgument<'a> {
    pub parameter: usize,
    pub names: &'a [String],
    pub origins: &'a [u32],
    pub complete: bool,
    pub translate: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassNameField<'a> {
    pub offset: u32,
    pub names: &'a [String],
    pub origins: &'a [u32],
    pub complete: bool,
    pub translate: bool,
}

/// Frozen queries consumed while Java expressions are still bound to IR values.
/// Aggregate queries agree across all occurrences; exact-site queries retain
/// meanings which are only established along one execution path.
pub struct Analysis {
    facts: HashMap<(ValueId, bool), Facts>,
    sites: HashMap<(ValueId, bool, Site), Facts>,
    literals: IdVec<ValueId, Facts>,
    switches: BTreeMap<BlockId, Facts>,
    packed: HashMap<ValueId, PackedConstant>,
    offsets: HashMap<ValueId, OffsetConstant>,
    local_names: HashMap<LocalId, String>,
    class_names: HashSet<ValueId>,
    shared_class_names: HashSet<ValueId>,
    class_name_groups: Vec<ClassNameGroup>,
    pub diagnostics: Vec<String>,
}

impl Analysis {
    pub fn analyze(
        ir: &MethodIr,
        program: &Program<'_>,
        mappings: &MappingPlan,
        member: MemberId,
    ) -> Self {
        infer::analyze(ir, program, mappings, member)
    }
    pub fn facts(&self, value: ValueId, root: bool) -> Facts {
        self.facts
            .get(&(value, root))
            .cloned()
            .unwrap_or_else(Facts::unknown)
    }
    pub fn facts_at(&self, value: ValueId, root: bool, site: Site) -> Facts {
        self.sites
            .get(&(value, root, site))
            .cloned()
            .unwrap_or_else(|| self.facts(value, root))
    }
    pub fn literal_domains(&self, value: ValueId) -> Vec<DomainId> {
        let Some(facts) = self
            .literals
            .get(value)
            .filter(|facts| !facts.unknown && facts.dependent.is_empty())
        else {
            return Vec::new();
        };
        let mut domains: Vec<_> = facts.domains.iter().copied().collect();
        domains.sort();
        domains
    }
    pub fn switch_domain(&self, block: BlockId) -> Option<DomainId> {
        self.switches.get(&block).and_then(Facts::domain)
    }
    pub fn packed_constant(&self, value: ValueId) -> Option<PackedConstant> {
        self.packed.get(&value).copied()
    }
    pub fn offset_constant(&self, value: ValueId) -> Option<OffsetConstant> {
        self.offsets.get(&value).copied()
    }
    pub fn local_name(&self, local: LocalId) -> Option<&str> {
        self.local_names.get(&local).map(String::as_str)
    }
    pub fn class_name_literal(&self, value: ValueId) -> bool {
        self.class_names.contains(&value)
    }
    /// The original literal is preserved for ordinary uses or because it joins
    /// another preserved literal at a translated class-name boundary.
    pub fn shared_class_name_literal(&self, value: ValueId) -> bool {
        self.shared_class_names.contains(&value)
    }
    pub fn class_name_groups(&self) -> &[ClassNameGroup] {
        &self.class_name_groups
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn into_class_name_groups(self) -> Vec<ClassNameGroup> {
        self.class_name_groups
    }
    fn class_name_inputs(&self) -> impl Iterator<Item = (&ClassNameInput, bool)> {
        self.class_name_groups.iter().flat_map(|group| {
            group.inputs.iter().map(move |input| {
                (
                    input,
                    input.complete && group.decision != ClassNameDecision::RewriteProducers,
                )
            })
        })
    }
    pub fn class_name_fields(&self) -> impl Iterator<Item = ClassNameField<'_>> {
        self.class_name_inputs().filter_map(|(input, translate)| {
            let ClassNameDestination::Field { offset } = input.destination else {
                return None;
            };
            Some(ClassNameField {
                offset,
                names: &input.names,
                origins: &input.origins,
                complete: input.complete,
                translate,
            })
        })
    }
    pub fn class_name_field(&self, offset: u32) -> Option<ClassNameField<'_>> {
        self.class_name_fields()
            .find(|field| field.offset == offset)
    }
    pub fn class_name_arguments(&self, offset: u32) -> impl Iterator<Item = ClassNameArgument<'_>> {
        self.class_name_inputs()
            .filter_map(move |(input, translate)| {
                let ClassNameDestination::Argument {
                    offset: pc,
                    parameter,
                } = input.destination
                else {
                    return None;
                };
                (pc == offset).then_some(ClassNameArgument {
                    parameter,
                    names: &input.names,
                    origins: &input.origins,
                    complete: input.complete,
                    translate,
                })
            })
    }
    pub fn class_name_consumers(&self) -> impl Iterator<Item = (u32, Vec<ClassNameArgument<'_>>)> {
        let mut arguments = BTreeMap::<_, Vec<_>>::new();
        for (input, translate) in self.class_name_inputs() {
            if let ClassNameDestination::Argument { offset, parameter } = input.destination {
                arguments
                    .entry(offset)
                    .or_default()
                    .push(ClassNameArgument {
                        parameter,
                        names: &input.names,
                        origins: &input.origins,
                        complete: input.complete,
                        translate,
                    });
            }
        }
        arguments.into_iter()
    }
}
