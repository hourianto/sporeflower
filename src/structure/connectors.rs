//! Remove execution-free source connectors after semantic ownership is proved.
use super::{FlowEnd, FlowGraph};
use crate::index::{BlockId, IdVec};

pub(super) fn contract(graph: &FlowGraph) -> Option<FlowGraph> {
    let count = graph.blocks.len();
    let mut handler_entry: IdVec<BlockId, _> = vec![false; count].into();
    for block in &graph.blocks {
        for handler in &block.handlers {
            handler_entry[handler.target] = true;
        }
    }
    let mut destination: IdVec<BlockId, _> = vec![None; count].into();
    for (node, block) in graph.blocks.iter_enumerated() {
        if !block.statements_empty
            || handler_entry[node]
            || !block.handlers.is_empty()
            || !matches!(block.end, FlowEnd::Goto(target) if target != node)
        {
            destination[node] = Some(node);
        }
    }
    let mut stamp: IdVec<BlockId, _> = vec![0; count].into();
    for root in graph.blocks.ids() {
        if destination[root].is_some() {
            continue;
        }
        let mut node = root;
        let mut path = Vec::new();
        while destination[node].is_none() && stamp[node] != root.index() + 1 {
            stamp[node] = root.index() + 1;
            path.push(node);
            let FlowEnd::Goto(target) = graph.blocks[node].end else {
                unreachable!()
            };
            node = target;
        }
        if let Some(target) = destination[node] {
            for node in path {
                destination[node] = Some(target);
            }
        } else {
            // A cycle of empty gotos still means nontermination. Leave it and
            // its entering connectors intact rather than invent a completion.
            for node in path {
                destination[node] = Some(node);
            }
        }
    }
    if destination
        .iter_enumerated()
        .all(|(node, target)| *target == Some(node))
    {
        return None;
    }
    let mut result = graph.clone();
    result.entry = destination[result.entry].unwrap();
    for (node, block) in result.blocks.iter_enumerated_mut() {
        if destination[node] != Some(node) {
            block.end = FlowEnd::Unreachable;
            block.duplication_cost = None;
        } else {
            block
                .end
                .map_destinations(|target| destination[target].unwrap());
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    const B0: crate::index::BlockId = crate::index::BlockId::from_index(0);
    const B1: crate::index::BlockId = crate::index::BlockId::from_index(1);
    const B2: crate::index::BlockId = crate::index::BlockId::from_index(2);
    const B99: crate::index::BlockId = crate::index::BlockId::from_index(99);
    use super::*;
    use crate::{cfg::HandlerEdge, structure::FlowBlock};

    fn graph(ends: Vec<FlowEnd>) -> FlowGraph {
        FlowGraph {
            entry: B0,
            blocks: ends
                .into_iter()
                .enumerate()
                .map(|(node, end)| FlowBlock {
                    pc: node as u32,
                    statements_empty: true,
                    duplication_cost: None,
                    is_loop_update: false,
                    end,
                    handlers: Vec::new(),
                    may_throw: false,
                })
                .collect(),
        }
    }

    #[test]
    fn logical_macro_exits_keep_their_identity() {
        let mut original = graph(vec![
            FlowEnd::Multi(vec![(B99, B1)]),
            FlowEnd::Goto(B2),
            FlowEnd::Return,
        ]);
        original.blocks[B0].statements_empty = false;
        let lowered = contract(&original).unwrap();
        assert!(
            matches!(&lowered.blocks[B0].end, FlowEnd::Multi(exits) if *exits == vec![(B99,B2)])
        );
        assert!(matches!(lowered.blocks[B1].end, FlowEnd::Unreachable));
    }

    #[test]
    fn exception_entry_and_opaque_macro_are_not_empty_connectors() {
        let mut original = graph(vec![FlowEnd::Return, FlowEnd::Goto(B2), FlowEnd::Return]);
        original.blocks[B0].handlers.push(HandlerEdge {
            target: B1,
            catch_type: None,
            table_index: 0,
        });
        assert!(contract(&original).is_none());
        original.blocks[B0].handlers.clear();
        original.blocks[B1].statements_empty = false;
        assert!(contract(&original).is_none());
    }

    #[test]
    fn empty_cycles_remain_nonterminating() {
        let original = graph(vec![
            FlowEnd::Goto(B1),
            FlowEnd::Goto(B2),
            FlowEnd::Goto(B1),
        ]);
        assert!(contract(&original).is_none());
    }
}
