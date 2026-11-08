//! Semantic regions are proved against immutable scheduled bytecode before
//! source handler eligibility or generic control-flow lowering is applied.
use super::{BlockId, FlowEnd, FlowGraph, HandlerFilter, Stmt, StructuredMethod, tree::Binder};
use crate::cfg::dispatch::{effective as effective_handlers, is_catch_all, same_dispatch};
use crate::index::IdVec;
use crate::{cfg::HandlerEdge, ir::MethodIr};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct RegionGraph {
    pub flow: FlowGraph,
    pub replacements: BTreeMap<BlockId, Vec<Stmt>>,
    pub cleanups: super::cleanup::CleanupFacts,
}

/// Potential normal completions of an opaque source region. Exceptions are
/// accounted for separately by its ordered handlers. In particular a return
/// is not a graph successor, and must never disappear behind a macro boundary.
#[derive(Default)]
pub(super) struct NormalCompletions {
    returns: BTreeSet<BlockId>,
    logical: BTreeSet<BlockId>,
    unbound: bool,
    fallthrough: bool,
}

impl NormalCompletions {
    fn of(body: &[Stmt]) -> Self {
        let mut result = Self {
            fallthrough: super::can_complete(body),
            ..Self::default()
        };
        result.visit(body, &mut Vec::new());
        result
    }

    /// Cleanup matching can follow explicit logical graph edges. Other normal
    /// completions need their own cleanup proof before this region can be
    /// enclosed in a newly recovered finally.
    pub fn represented_by_edges(&self, end: &FlowEnd) -> bool {
        if !self.returns.is_empty() || self.unbound {
            return false;
        }
        let logical: BTreeSet<_> = match end {
            FlowEnd::Multi(exits) => exits.iter().map(|(logical, _)| *logical).collect(),
            _ => BTreeSet::new(),
        };
        self.logical.is_subset(&logical) && (!self.fallthrough || matches!(end, FlowEnd::Goto(_)))
    }

    fn visit(&mut self, body: &[Stmt], scope: &mut Vec<Binder>) {
        for statement in body {
            let bound_label = |target| {
                scope.iter().any(|binder| {
                    !matches!(binder, Binder::Dispatch(_)) && binder.region() == target
                })
            };
            // Keep exit classification exhaustive. Child ownership comes from
            // the statement shape, including opaque-region exit bindings.
            match statement {
                Stmt::Return(block) => {
                    self.returns.insert(*block);
                }
                Stmt::Jump(target) => {
                    self.logical.insert(*target);
                }
                Stmt::Break { target } => self.unbound |= !bound_label(*target),
                Stmt::Continue { header } => {
                    self.unbound |= !scope.iter().any(
                        |binder| matches!(binder, Binder::Loop { header: owner, .. } if owner == header),
                    );
                }
                Stmt::DispatchContinue { region, .. } => {
                    self.unbound |= !scope.contains(&Binder::Dispatch(*region));
                }
                Stmt::BoundRegion { .. } => self.unbound = true,
                Stmt::Dispatch { cases, exits, .. } => {
                    self.unbound |= exits.iter().any(|&(_, region)| !bound_label(region));
                    self.returns.extend(cases.iter().filter_map(|case| {
                        matches!(case.end, FlowEnd::Return).then_some(case.head)
                    }));
                }
                Stmt::Basic(_)
                | Stmt::Statements { .. }
                | Stmt::Throw(_)
                | Stmt::Unreachable(_)
                | Stmt::If { .. }
                | Stmt::Loop { .. }
                | Stmt::Label { .. }
                | Stmt::Switch { .. }
                | Stmt::Try { .. }
                | Stmt::TryFinally { .. }
                | Stmt::Synchronized { .. } => (),
            }
            let binder = statement.binder();
            scope.extend(binder);
            // Abrupt cleanup replaces all pending normal transfers from the
            // protected body; only the cleanup's own completions survive.
            if let Stmt::TryFinally { cleanup, .. } = statement
                && !super::can_complete(cleanup)
            {
                self.visit(cleanup, scope);
            } else {
                for body in statement.bodies() {
                    self.visit(body, scope);
                }
            }
            if binder.is_some() {
                scope.pop();
            }
            if !super::can_complete(std::slice::from_ref(statement)) {
                break;
            }
        }
    }
}

/// A proof owns every consumed node: external normal/exception entries may
/// reach only `entry`, and all outward transfers are represented by `end` and
/// logical Jump bindings in `body`. The body is already composed with any
/// previously proved regions; installation never expands it again. Handler
/// order remains significant.
pub(super) struct VerifiedRegion {
    pub entry: BlockId,
    pub nodes: Vec<BlockId>,
    pub outer: Vec<HandlerEdge>,
    pub end: FlowEnd,
    pub body: Vec<Stmt>,
    pub may_throw: bool,
    pub cleanups: super::cleanup::CleanupFacts,
}

/// Discovery facts only. The cleanup verifier proves the protected scope,
/// exact statement slices, copy boundaries, and ownership jointly.
pub(super) struct CleanupCandidate {
    pub entry: BlockId,
    pub handler: BlockId,
    /// All live throwing sources lexically covered by this original handler,
    /// including sources where an earlier catch shadows its dispatch. A
    /// whole-scope proof must cover every member; a shared witness can certify
    /// a subset without consuming the original handler.
    pub covered: Vec<BlockId>,
    pub outer: Vec<HandlerEdge>,
}

pub(super) fn cleanup_candidates(ir: &MethodIr, graph: &FlowGraph) -> Vec<CleanupCandidate> {
    if graph.blocks.is_empty() {
        return Vec::new();
    }
    let effective: IdVec<BlockId, _> = graph
        .blocks
        .iter()
        .map(|block| {
            effective_handlers(&block.handlers)
                .cloned()
                .collect::<Vec<_>>()
        })
        .collect();
    let successors: IdVec<BlockId, _> = graph
        .blocks
        .iter_enumerated()
        .map(|(node, block)| {
            let mut targets = block.end.successors();
            if block.may_throw {
                targets.extend(effective[node].iter().map(|handler| handler.target));
            }
            targets.sort_unstable();
            targets.dedup();
            targets
        })
        .collect();
    let mut predecessors: IdVec<BlockId, _> = vec![Vec::new(); graph.blocks.len()].into();
    for (source, targets) in successors.iter_enumerated() {
        for &target in targets {
            predecessors[target].push(source);
        }
    }
    let dominance = super::analysis::Dominators::new(&successors, &predecessors, graph.entry);
    let handlers: BTreeSet<_> = effective
        .iter_enumerated()
        .filter(|(node, _)| dominance.idom[*node].is_some())
        .flat_map(|(_, handlers)| handlers.iter())
        .filter(|handler| is_catch_all(handler.catch_type.as_deref()))
        .map(|handler| handler.target)
        .collect();
    let mut result = Vec::new();
    for handler in handlers {
        let Some(caught) = ir
            .blocks
            .get(handler)
            .and_then(|block| block.caught_exception)
        else {
            continue;
        };
        let Some(dispatched_entry) = dominance.idom[handler].filter(|entry| *entry != handler)
        else {
            continue;
        };
        let covered: Vec<_> = graph
            .blocks
            .iter_enumerated()
            .filter_map(|(node, block)| {
                (dominance.idom[node].is_some()
                    && block.may_throw
                    && block.handlers.iter().any(|edge| edge.target == handler))
                .then_some(node)
            })
            .collect();
        if covered.is_empty() {
            continue;
        }
        // Exception dominance describes execution, not lexical ownership:
        // `catch (Throwable)` shadows the finally on the ordinary body, so
        // the cleanup handler's immediate dominator can be that catch alone.
        // Start at the common owner of every original table membership. The
        // verifier still follows effective dispatch and proves every exit.
        let lexical = graph.blocks.iter_enumerated().filter_map(|(node, block)| {
            (dominance.idom[node].is_some()
                && node != handler
                && block.handlers.iter().any(|edge| edge.target == handler))
            .then_some(node)
        });
        let Some(entry) = dominance.common(std::iter::once(dispatched_entry).chain(lexical)) else {
            continue;
        };
        let mut cleanup: IdVec<BlockId, _> = vec![false; graph.blocks.len()].into();
        let mut completions = Vec::new();
        let mut pending = vec![handler];
        while let Some(node) = pending.pop() {
            if std::mem::replace(&mut cleanup[node], true) {
                continue;
            }
            if matches!(
                ir.blocks.get(node).map(|block| &block.terminator),
                Some(crate::ir::Terminator::Return(_))
            ) || matches!(ir.blocks.get(node).map(|block| &block.terminator), Some(crate::ir::Terminator::Throw { value, .. }) if *value == caught)
            {
                completions.push(node);
            } else {
                pending.extend(graph.blocks[node].end.successors());
            }
        }
        let Some(&completion) = completions.first() else {
            continue;
        };
        let outer = effective[completion].clone();
        if completions
            .iter()
            .any(|&node| !same_dispatch(&effective[node], &outer))
        {
            continue;
        }
        result.push(CleanupCandidate {
            entry,
            handler,
            covered,
            outer,
        });
    }
    // Descendant entries are considered before enclosing entries. When two
    // candidates share an entry, the verifier first tries the entire original
    // handler scope before considering a smaller scope with a shared witness.
    result.sort_by_key(|candidate| std::cmp::Reverse(dominance.preorder[candidate.entry]));
    result
}

impl RegionGraph {
    pub fn normal_completions(&self, entry: BlockId) -> Option<NormalCompletions> {
        self.replacements
            .get(&entry)
            .map(|body| NormalCompletions::of(body))
    }

    pub fn new(ir: &MethodIr) -> Self {
        Self {
            flow: FlowGraph::from(ir),
            replacements: BTreeMap::new(),
            cleanups: super::cleanup::CleanupFacts::default(),
        }
    }

    pub fn install(&mut self, mut region: VerifiedRegion) {
        // A single logical continuation is ordinary fallthrough once its
        // transfers are bound inside the region. Keep that closed shape in
        // the graph so enclosing cleanup proofs can match it as one operation.
        // Preserve the binding until source continuation projection: a later
        // return may belong before cleanup when its saved value is stable.
        if let FlowEnd::Multi(exits) = &region.end
            && let [(logical, target)] = exits.as_slice()
        {
            region.body = super::bindings::bind(region.body, &[(*logical, Vec::new())]);
            region.end = FlowEnd::Goto(*target);
        }
        self.cleanups.extend(region.cleanups);
        super::installation::install(
            &mut self.flow,
            &mut self.replacements,
            super::installation::Region {
                entry: region.entry,
                nodes: region.nodes,
                outer: region.outer,
                end: region.end,
                body: super::installation::Body::Composed(region.body),
                may_throw: region.may_throw,
            },
            None,
        );
    }

    pub fn recover_cleanups(
        &mut self,
        ir: &MethodIr,
        mut prove: impl FnMut(&Self, &CleanupCandidate) -> Option<Vec<VerifiedRegion>>,
    ) {
        let mut recovered = BTreeSet::new();
        loop {
            let plan = cleanup_candidates(ir, &self.flow)
                .into_iter()
                .filter(|candidate| !recovered.contains(&candidate.handler))
                .find_map(|candidate| {
                    prove(self, &candidate).map(|regions| (candidate.handler, regions))
                });
            let Some((handler, regions)) = plan else {
                break;
            };
            debug_assert!(!regions.is_empty());
            let complete = regions.iter().any(|region| region.nodes.contains(&handler));
            debug_assert!(regions.iter().all(|region| region.entry != handler));
            // Every certificate removes protected memberships. A complete
            // scope also retires its handler; a shared witness remains eligible
            // until every remaining protected path has its own certificate.
            // Recompute ownership after composing the child's semantic macro.
            if complete {
                recovered.insert(handler);
            }
            for region in regions {
                self.install(region);
            }
        }
    }

    pub fn lower(self, filter: Option<&HandlerFilter<'_>>) -> crate::Result<StructuredMethod> {
        let eligibility =
            super::eligibility::Eligibility::new(&self.flow, &self.replacements, filter);
        let mut result = super::structure_graph_inner(&self.flow, &eligibility)?;
        result.body = super::installation::expand(result.body, &self.replacements);
        Ok(result)
    }
}
