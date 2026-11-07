//! Source catch eligibility, separate from the ordered dispatch used for ownership.
use super::{BlockId, FlowGraph, HandlerFilter, Stmt};
use crate::cfg::HandlerEdge;
use crate::cfg::dispatch::{effective, same_handler};
use std::{collections::BTreeMap, rc::Rc};

#[cfg(test)]
mod tests;

#[derive(Clone, Default)]
pub(super) struct Eligibility<'a> {
    filter: Option<&'a HandlerFilter<'a>>,
    allowed: crate::index::IdVec<BlockId, Vec<HandlerEdge>>,
    effects: BTreeMap<BlockId, Rc<[Stmt]>>,
}

impl<'a> Eligibility<'a> {
    pub fn new(
        graph: &FlowGraph,
        effects: &BTreeMap<BlockId, Vec<Stmt>>,
        filter: Option<&'a HandlerFilter<'a>>,
    ) -> Self {
        let Some(filter) = filter else {
            return Self::default();
        };
        Self {
            filter: Some(filter),
            allowed: graph
                .blocks
                .iter_enumerated()
                .map(|(node, block)| {
                    // Pure connectors retain lexical outer coverage, which
                    // can differ from the throwing body's dispatch. That is
                    // ownership evidence, not an effect authorizing a catch:
                    // an outer unchecked subtype may be shadowed at every
                    // actual operation selected into the region.
                    if !block.may_throw || block.handlers.is_empty() {
                        Vec::new()
                    } else {
                        filter(node, effects.get(&node).map(Vec::as_slice), &block.handlers)
                    }
                })
                .collect(),
            effects: effects
                .iter()
                .map(|(&node, body)| (node, Rc::from(body.clone())))
                .collect(),
        }
    }

    /// Union effect eligibility only within this owned body. Disconnected
    /// regions with the same dispatch signature cannot authorize each other.
    pub fn caught(&self, nodes: &[BlockId], handlers: &[HandlerEdge]) -> Vec<HandlerEdge> {
        if self.filter.is_none() {
            return effective(handlers).cloned().collect();
        }
        effective(handlers)
            .filter(|handler| {
                nodes.iter().any(|&node| {
                    self.allowed.get(node).is_some_and(|allowed| {
                        allowed.iter().any(|edge| same_handler(edge, handler))
                    })
                })
            })
            .cloned()
            .collect()
    }

    /// Refresh before source composition, using the same explicit body state.
    /// Effect replacements can include semantic regions deferred by the source
    /// overlay; each original identity must still be expanded only once.
    pub fn install(
        &mut self,
        entry: BlockId,
        body: &super::installation::Body,
        outer: &[HandlerEdge],
        consumed: &[BlockId],
    ) {
        let Some(filter) = self.filter else {
            return;
        };
        let effects = body.clone().compose(&self.effects);
        for &node in consumed {
            if let Some(allowed) = self.allowed.get_mut(node) {
                allowed.clear();
            }
            self.effects.remove(&node);
        }
        if !self.allowed.contains_id(entry) {
            self.allowed.resize(entry.index() + 1, Vec::new());
        }
        self.allowed[entry] = filter(entry, Some(&effects), outer);
        self.effects.insert(entry, Rc::from(effects));
    }

    pub fn boundary(&mut self, block: BlockId) {
        if self.filter.is_some() {
            if !self.allowed.contains_id(block) {
                self.allowed.resize(block.index() + 1, Vec::new());
            }
            self.effects.insert(block, Rc::from(Vec::<Stmt>::new()));
        }
    }

    /// A dispatcher emits separate try bodies, so each case has its own scope.
    pub fn dispatch_graph(&self, graph: &FlowGraph) -> FlowGraph {
        let mut result = graph.clone();
        for block in &mut result.blocks {
            block.handlers = effective(&block.handlers).cloned().collect();
        }
        if self.filter.is_some() {
            for (node, block) in result.blocks.iter_enumerated_mut() {
                block.handlers.retain(|handler| {
                    self.allowed.get(node).is_some_and(|allowed| {
                        allowed.iter().any(|edge| same_handler(edge, handler))
                    })
                });
            }
        }
        result
    }
}
