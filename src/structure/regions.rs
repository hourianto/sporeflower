//! Semantic regions are proved against immutable scheduled bytecode before
//! source handler eligibility or generic control-flow lowering is applied.
use super::{BlockId, FlowEnd, FlowGraph, HandlerFilter, Stmt, StructuredMethod};
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
        result.visit(body, &mut Vec::new(), &mut Vec::new(), &mut Vec::new());
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

    // Keep this match exhaustive: adding a macro/statement kind requires an
    // explicit decision about the normal exits it can conceal.
    fn visit(
        &mut self,
        body: &[Stmt],
        labels: &mut Vec<usize>,
        loops: &mut Vec<BlockId>,
        dispatches: &mut Vec<usize>,
    ) {
        for statement in body {
            match statement {
                Stmt::Basic(_)
                | Stmt::Statements { .. }
                | Stmt::Throw(_)
                | Stmt::Unreachable(_) => (),
                Stmt::Return(block) => {
                    self.returns.insert(*block);
                }
                Stmt::Jump(target) => {
                    self.logical.insert(*target);
                }
                Stmt::Break { target } => self.unbound |= !labels.contains(target),
                Stmt::Continue { header } => self.unbound |= !loops.contains(header),
                Stmt::DispatchContinue { region, .. } => {
                    self.unbound |= !dispatches.contains(region)
                }
                Stmt::BoundRegion { exits, .. } => {
                    self.unbound = true;
                    for (_, body) in exits {
                        self.visit(body, labels, loops, dispatches);
                    }
                }
                Stmt::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    self.visit(then_body, labels, loops, dispatches);
                    self.visit(else_body, labels, loops, dispatches);
                }
                Stmt::Loop {
                    region,
                    header,
                    body,
                    // Its certified continuation is a nonthrowing local
                    // update, so it adds no outward normal completion.
                    ..
                } => {
                    labels.push(*region);
                    loops.push(*header);
                    self.visit(body, labels, loops, dispatches);
                    loops.pop();
                    labels.pop();
                }
                Stmt::Label { region, body } => {
                    labels.push(*region);
                    self.visit(body, labels, loops, dispatches);
                    labels.pop();
                }
                Stmt::Switch { region, cases, .. } => {
                    labels.push(*region);
                    for case in cases {
                        self.visit(&case.body, labels, loops, dispatches);
                    }
                    labels.pop();
                }
                Stmt::Try { body, catches } => {
                    self.visit(body, labels, loops, dispatches);
                    for catch in catches {
                        self.visit(&catch.body, labels, loops, dispatches);
                    }
                }
                Stmt::TryFinally { body, cleanup, .. } => {
                    if super::can_complete(cleanup) {
                        self.visit(body, labels, loops, dispatches);
                    }
                    self.visit(cleanup, labels, loops, dispatches);
                }
                Stmt::Synchronized { body, .. } => self.visit(body, labels, loops, dispatches),
                Stmt::Dispatch {
                    region,
                    cases,
                    exits,
                    ..
                } => {
                    dispatches.push(*region);
                    self.unbound |= exits.iter().any(|(_, region)| !labels.contains(region));
                    for case in cases {
                        self.visit(&case.body, labels, loops, dispatches);
                        if matches!(case.end, FlowEnd::Return) {
                            self.returns.insert(case.head);
                        }
                    }
                    dispatches.pop();
                }
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
    /// candidate must not certify only one same-handler fragment.
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
        let Some(entry) =
            dominance.common(std::iter::once(dispatched_entry).chain(covered.iter().copied()))
        else {
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
    // candidates share an entry, the verifier still requires the entire
    // original handler scope; an unresolved inner cleanup cannot be accepted
    // as an independently completed fragment of its outer cleanup.
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

    pub fn install(&mut self, region: VerifiedRegion) {
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
            debug_assert!(
                regions.iter().any(|region| region.nodes.contains(&handler))
                    && regions.iter().all(|region| region.entry != handler)
            );
            // A successful certificate consumes one original handler identity.
            // Recompute ownership after composition so an outer scope observes
            // the child's semantic macro and its effective outer handlers.
            recovered.insert(handler);
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
