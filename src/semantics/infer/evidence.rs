//! Read-only occurrence facts. Deferred meanings are resolved at the use site;
//! packed layouts prefer the producer-only snapshot from the first solve.
use super::super::{
    Facts,
    flow::{Graph, NodeId},
};
use crate::mappings::{ConditionalDomain, DomainId, MappingPlan};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::semantics) enum Dependent {
    Conditional {
        selector: NodeId,
        cases: Vec<ConditionalDomain>,
    },
    Packed {
        source: NodeId,
        cases: Vec<(i64, i64, Option<DomainId>)>,
    },
}

#[derive(Clone, Copy)]
pub(super) struct Evidence<'a> {
    pub graph: &'a Graph,
    pub ranges: &'a super::ranges::Queries<'a>,
    pub mappings: &'a MappingPlan,
    pub facts: &'a [Facts],
    pub layout: &'a [Facts],
}

impl Evidence<'_> {
    pub(super) fn resolve(&self, at: NodeId, mut facts: Facts, final_use: bool) -> Facts {
        let dependent = std::mem::take(&mut facts.dependent);
        for dependency in dependent {
            let mut unresolved = false;
            let mut selected = false;
            match &dependency {
                Dependent::Conditional { selector, cases } => {
                    if let Some(domain) = self.conditional(at, *selector, cases) {
                        facts.domains.insert(domain);
                        selected = true;
                    } else {
                        unresolved = self.graph.known_at(at, *selector).is_none();
                    }
                }
                Dependent::Packed { source, cases } => {
                    for &(mask, expected, domain) in cases {
                        match if mask == 0 {
                            Some(0)
                        } else {
                            self.graph.masked_at(at, *source, mask)
                        } {
                            Some(value) if value == expected => {
                                selected = true;
                                if let Some(domain) = domain {
                                    facts.domains.insert(domain);
                                } else {
                                    facts.unknown = true;
                                }
                            }
                            None => unresolved = true,
                            _ => {}
                        }
                    }
                }
            }
            if !selected {
                if unresolved && !final_use {
                    facts.dependent.insert(dependency);
                } else {
                    facts.unknown = true;
                }
            }
        }
        facts
    }
    pub(super) fn at(&self, value: NodeId, at: NodeId) -> Facts {
        self.resolve(at, self.facts[value].clone(), false)
    }
    pub(super) fn final_at(&self, value: NodeId, at: NodeId) -> Facts {
        self.resolve(at, self.facts[value].clone(), true)
    }
    pub(super) fn conditional(
        &self,
        at: NodeId,
        selector: NodeId,
        cases: &[ConditionalDomain],
    ) -> Option<DomainId> {
        let known = self.graph.known_at(at, selector);
        let mut selected = HashSet::new();
        for case in cases {
            let active = if case.otherwise {
                cases
                    .iter()
                    .filter_map(|case| case.equals)
                    .all(|number| self.graph.excludes_at(at, selector, number))
            } else if let Some(number) = case.equals {
                known == Some(number)
            } else if let Some(number) = case.not_equals {
                self.graph.excludes_at(at, selector, number)
            } else {
                false
            };
            if active {
                selected.insert(case.domain);
            }
        }
        (selected.len() == 1).then(|| *selected.iter().next().unwrap())
    }

    pub(super) fn layout_source(&self, source: NodeId, at: NodeId) -> Facts {
        let established = &self.layout[source];
        if !established.unknown
            && established
                .domains
                .iter()
                .any(|&domain| !self.mappings.domain(domain).bit_fields.is_empty())
        {
            self.resolve(at, established.clone(), false)
        } else {
            self.at(source, at)
        }
    }
}
