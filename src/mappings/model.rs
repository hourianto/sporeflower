use crate::hash::{HashMap, HashSet};
use crate::{
    naming::NameOverrides,
    program::{ClassId, MemberId},
};
use std::collections::BTreeMap;

/// A semantic domain identity, deliberately distinct from JVM class identities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DomainId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Target {
    Field(MemberId),
    Return(MemberId),
    Parameter { method: MemberId, index: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CallSite {
    pub method: MemberId,
    pub offset: u32,
    pub parameter: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DomainKind {
    Value,
    Flags,
    Slots,
    Packed,
    Numeric,
    String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ConstantValue {
    Integral(i64),
    String(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DomainValue {
    pub name: String,
    pub descriptor: String,
    pub value: ConstantValue,
    /// Original field identity for a bytecode constant; None for a generated constant.
    pub source: Option<MemberId>,
    pub slot_domain: Option<DomainId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BitField {
    pub domain: Option<DomainId>,
    pub shift: u8,
    pub bits: u8,
    pub signed: bool,
    pub selector_mask: i64,
    pub selector_value: i64,
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum NumberFormat {
    Rgb,
    Argb,
    Fixed {
        fraction_bits: u8,
        unit: Option<String>,
    },
    Scaled {
        divisor: i64,
        unit: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Domain {
    /// The generated source owner, in slash-separated JVM form.
    pub owner: String,
    pub kind: DomainKind,
    pub values: Vec<DomainValue>,
    pub exclusive_masks: Vec<i64>,
    pub bit_fields: Vec<BitField>,
    pub format: Option<NumberFormat>,
}

impl Domain {
    pub fn generated(&self) -> bool {
        self.values.iter().any(|value| value.source.is_none())
            || self.bit_fields.iter().any(|field| field.name.is_some())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ArrayDimension {
    pub index: Option<DomainId>,
    pub slots: Option<DomainId>,
    pub records: Option<RecordLayout>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RecordLayout {
    pub domain: DomainId,
    pub stride: u32,
    pub offset: u32,
    pub planes: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ContainerContract {
    pub elements: Option<DomainId>,
    pub keys: Option<DomainId>,
    pub values: Option<DomainId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ConditionalDomain {
    pub parameter: usize,
    pub domain: DomainId,
    pub equals: Option<i64>,
    pub not_equals: Option<i64>,
    pub otherwise: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DerivedDomain {
    Parameter(usize),
    Slot {
        parameter: usize,
        dimension: usize,
        slot: u32,
    },
    Conditional(Vec<ConditionalDomain>),
}

/// One complete contract. An explicit override replaces this entire value.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Contract {
    pub leaf: Option<DomainId>,
    pub arrays: BTreeMap<usize, ArrayDimension>,
    pub container: Option<ContainerContract>,
    pub derived: Option<DerivedDomain>,
    pub class_name: bool,
}

impl Contract {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// Resolved mapping input. Semantic contracts are constructed only by resolution;
/// name overrides are validated separately by `NamePlan`. All keys use original identities.
#[derive(Clone, Debug, Default)]
pub struct MappingPlan {
    pub names: NameOverrides,
    pub origins: super::MappingOrigins,
    /// Actionable authored-input warnings, retained even when a binding is conservatively omitted.
    pub diagnostics: Vec<super::MappingDiagnostic>,
    pub(super) domains: Vec<Domain>,
    pub(super) bindings: HashMap<Target, Contract>,
    pub(super) real_values: HashMap<MemberId, DomainId>,
    pub(super) calls: HashMap<CallSite, DomainId>,
    pub(super) ignored_classes: HashSet<ClassId>,
}

impl MappingPlan {
    pub fn domains(&self) -> &[Domain] {
        &self.domains
    }
    pub fn bindings(&self) -> &HashMap<Target, Contract> {
        &self.bindings
    }
    pub fn calls(&self) -> &HashMap<CallSite, DomainId> {
        &self.calls
    }
    pub fn real_value(&self, field: MemberId) -> Option<DomainId> {
        self.real_values.get(&field).copied()
    }
    pub fn ignores(&self, class: ClassId) -> bool {
        self.ignored_classes.contains(&class)
    }
    pub fn domain(&self, id: DomainId) -> &Domain {
        &self.domains[id.0]
    }
    pub fn contract(&self, target: Target) -> Option<&Contract> {
        self.bindings.get(&target)
    }
    pub fn call_domain(
        &self,
        method: MemberId,
        offset: u32,
        parameter: Option<usize>,
    ) -> Option<DomainId> {
        self.calls
            .get(&CallSite {
                method,
                offset,
                parameter,
            })
            .copied()
    }
    /// Keep author names while disabling all semantic presentation contracts.
    pub fn without_semantics(mut self) -> Self {
        self.domains.clear();
        self.bindings.clear();
        self.real_values.clear();
        self.calls.clear();
        self.names.reserved_classes.clear();
        self
    }
}

#[derive(Clone, Debug)]
pub struct MappingSource {
    pub path: String,
    pub source: String,
    pub builtin: bool,
}

impl MappingSource {
    pub fn new(path: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            source: source.into(),
            builtin: false,
        }
    }
}
