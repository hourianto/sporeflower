//! One deterministic predecessor per inferred fact. The propagation graph may
//! contain cycles, but a fact's first-arrival chain always points backward.
use crate::program::MemberId;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InferredEffect {
    Declared(String),
    Hidden(String),
    Unknown,
    MayInstantiationException,
    MayIllegalAccessException,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectSite {
    Declaration,
    Call(u32),
    Throw(u32),
    HandlerBlock(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectStep {
    pub member: MemberId,
    pub site: EffectSite,
    pub effect: InferredEffect,
}

pub(super) struct NodeProvenance {
    pub member: MemberId,
    pub site: EffectSite,
    pub causes: BTreeMap<InferredEffect, usize>,
}

impl NodeProvenance {
    pub fn new(member: MemberId, site: EffectSite) -> Self {
        Self {
            member,
            site,
            causes: BTreeMap::new(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Trace {
    step: EffectStep,
    parent: Option<usize>,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub(super) struct Provenance {
    traces: Vec<Trace>,
    methods: HashMap<(MemberId, InferredEffect), usize>,
}

impl Provenance {
    pub fn record(&mut self, step: EffectStep, parent: Option<usize>) -> usize {
        let id = self.traces.len();
        self.traces.push(Trace { step, parent });
        id
    }

    pub fn method(&mut self, member: MemberId, effect: InferredEffect, trace: usize) {
        self.methods.insert((member, effect), trace);
    }

    pub fn explain(&self, member: MemberId, effect: &InferredEffect) -> Vec<EffectStep> {
        let mut result = Vec::new();
        let mut trace = self.methods.get(&(member, effect.clone())).copied();
        while let Some(id) = trace {
            let entry = &self.traces[id];
            result.push(entry.step.clone());
            trace = entry.parent;
        }
        result.reverse();
        result
    }
}
