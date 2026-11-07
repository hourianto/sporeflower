//! Install a proved region as one opaque graph node. Proofs and effect policies
//! belong to the callers; graph replacement and macro composition happen here.
use super::{BlockId, FlowBlock, FlowEnd, FlowGraph, Stmt};
use crate::cfg::HandlerEdge;
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
mod tests;

/// A composed body may still contain Basic(entry), for example the acquisition
/// prefix of a monitor. That is an original operation, not another macro use.
/// Re-expanding it would duplicate the region or reintroduce consumed cleanup.
#[derive(Clone)]
pub(super) enum Body {
    Unexpanded(Vec<Stmt>),
    Composed(Vec<Stmt>),
}

impl Body {
    pub fn compose<T: AsRef<[Stmt]>>(self, replacements: &BTreeMap<BlockId, T>) -> Vec<Stmt> {
        match self {
            Self::Unexpanded(body) => expand(body, replacements),
            Self::Composed(body) => body,
        }
    }
}

/// The caller proves that outside entries reach only entry, and supplies every
/// normal completion (including logical exit identities) and ordered handler.
pub(super) struct Region {
    pub entry: BlockId,
    pub nodes: Vec<BlockId>,
    pub outer: Vec<HandlerEdge>,
    pub end: FlowEnd,
    pub body: Body,
    pub may_throw: bool,
}

pub(super) fn install(
    graph: &mut FlowGraph,
    replacements: &mut BTreeMap<BlockId, Vec<Stmt>>,
    region: Region,
    eligibility: Option<&mut super::eligibility::Eligibility<'_>>,
) {
    debug_assert!(region.nodes.contains(&region.entry));
    // Refresh effects against the old overlay, before retiring consumed nodes.
    // Effects can include semantic bodies which source expansion defers until
    // the outer lowering boundary, so the two maps remain separate views. With
    // no eligibility here, RegionGraph builds it from composed bodies at lower.
    if let Some(eligibility) = eligibility {
        eligibility.install(region.entry, &region.body, &region.outer, &region.nodes);
    }
    let body = region.body.compose(replacements);
    let consumed: BTreeSet<_> = region.nodes.into_iter().collect();
    for &node in &consumed {
        tombstone(&mut graph.blocks[node]);
    }
    let entry = &mut graph.blocks[region.entry];
    entry.end = region.end;
    entry.handlers = region.outer;
    entry.may_throw = region.may_throw;
    replacements.retain(|node, _| !consumed.contains(node));
    replacements.insert(region.entry, body);
}

/// Consumed nodes and excluded subgraph nodes cannot participate in flow or be
/// mistaken for empty connectors, duplicable blocks, or loop updates.
pub(super) fn tombstone(block: &mut FlowBlock) {
    block.end = FlowEnd::Unreachable;
    block.handlers.clear();
    block.may_throw = false;
    block.statements_empty = false;
    block.duplication_cost = None;
    block.is_loop_update = false;
}

/// Each case is its own source catch scope. Callers expand their own overlay
/// afterward; the graph still supplies logical exits and original terminators.
pub(super) fn dispatch(
    graph: &FlowGraph,
    eligibility: &super::eligibility::Eligibility<'_>,
) -> Vec<Stmt> {
    let graph = eligibility.dispatch_graph(graph);
    let cases = super::analysis::reachable_with_handlers(&graph)
        .into_iter()
        .map(|head| super::DispatchCase {
            head,
            body: vec![Stmt::Basic(head)],
            end: graph.blocks[head].end.clone(),
            handlers: if graph.blocks[head].may_throw {
                graph.blocks[head].handlers.clone()
            } else {
                Vec::new()
            },
        })
        .collect();
    vec![Stmt::Dispatch {
        region: 0,
        entry: graph.entry,
        cases,
        exits: Vec::new(),
    }]
}

/// Expand only graph placeholders. Inserted bodies are already composed and
/// must not be traversed again, even when they retain their own entry identity.
pub(super) fn expand<T: AsRef<[Stmt]>>(
    body: Vec<Stmt>,
    macros: &BTreeMap<BlockId, T>,
) -> Vec<Stmt> {
    let mut result = Vec::new();
    for stmt in body {
        match stmt {
            Stmt::BoundRegion { head, exits } if macros.contains_key(&head) => {
                result.extend(super::bindings::bind(
                    macros[&head].as_ref().to_vec(),
                    &exits,
                ));
            }
            Stmt::Basic(block) if macros.contains_key(&block) => {
                result.extend(macros[&block].as_ref().iter().cloned())
            }
            Stmt::Return(block) | Stmt::Throw(block) | Stmt::Unreachable(block)
                if macros.contains_key(&block) => {}
            Stmt::Dispatch {
                region,
                entry,
                cases,
                exits,
            } => {
                let heads: BTreeSet<_> = cases.iter().map(|case| case.head).collect();
                let cases = cases
                    .into_iter()
                    .map(|mut case| {
                        case.body = expand(case.body, macros);
                        if macros.contains_key(&case.head)
                            && matches!(
                                case.end,
                                FlowEnd::Return | FlowEnd::Throw | FlowEnd::Unreachable
                            )
                        {
                            case.end = FlowEnd::Unreachable;
                        }
                        if let FlowEnd::Multi(destinations) = &case.end {
                            let bindings = destinations
                                .iter()
                                .map(|&(logical, target)| {
                                    let transfer = if heads.contains(&target) {
                                        vec![Stmt::DispatchContinue { region, target }]
                                    } else if let Some(&(_, label)) =
                                        exits.iter().find(|&&(block, _)| block == target)
                                    {
                                        vec![Stmt::Break { target: label }]
                                    } else {
                                        vec![Stmt::Jump(target)]
                                    };
                                    (logical, transfer)
                                })
                                .collect::<Vec<_>>();
                            case.body = super::bindings::bind(case.body, &bindings);
                        }
                        case
                    })
                    .collect();
                result.push(Stmt::Dispatch {
                    region,
                    entry,
                    cases,
                    exits,
                });
            }
            other => result.push(other.map_bodies(|body| expand(body, macros))),
        }
    }
    result
}
