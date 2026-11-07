const B0: crate::index::BlockId = crate::index::BlockId::from_index(0);
const B1: crate::index::BlockId = crate::index::BlockId::from_index(1);
const B2: crate::index::BlockId = crate::index::BlockId::from_index(2);
const B3: crate::index::BlockId = crate::index::BlockId::from_index(3);
const B4: crate::index::BlockId = crate::index::BlockId::from_index(4);
const B5: crate::index::BlockId = crate::index::BlockId::from_index(5);
const B13: crate::index::BlockId = crate::index::BlockId::from_index(13);
use super::*;

fn shared_catches(prefix: bool, count: usize) -> FlowGraph {
    let join = count;
    let caught = count + 1;
    let outer = count + 2;
    let entry = count + 3;
    let throw = if prefix { count + 4 } else { caught };
    let catches = vec![
        HandlerEdge {
            target: BlockId::from_index(caught),
            catch_type: Some("java/lang/Exception".into()),
            table_index: 0,
        },
        HandlerEdge {
            target: BlockId::from_index(outer),
            catch_type: None,
            table_index: 1,
        },
    ];
    let block = |id, end, handlers, may_throw| FlowBlock {
        pc: id as u32,
        end,
        handlers,
        may_throw,
        statements_empty: false,
        duplication_cost: None,
        is_loop_update: false,
    };
    let mut blocks: Vec<_> = (0..count)
        .map(|id| {
            block(
                id,
                FlowEnd::Goto(BlockId::from_index(join)),
                catches.clone(),
                true,
            )
        })
        .collect();
    blocks.push(block(join, FlowEnd::Return, vec![], false));
    let outer_coverage = vec![HandlerEdge {
        target: BlockId::from_index(outer),
        catch_type: None,
        table_index: 2,
    }];
    blocks.push(if prefix {
        block(
            caught,
            FlowEnd::Goto(BlockId::from_index(throw)),
            vec![],
            false,
        )
    } else {
        block(caught, FlowEnd::Throw, outer_coverage.clone(), true)
    });
    blocks.push(block(outer, FlowEnd::Throw, vec![], true));
    blocks.push(block(
        entry,
        FlowEnd::Switch {
            cases: (0..count - 1)
                .map(|id| (id as i32, BlockId::from_index(id)))
                .collect(),
            default: BlockId::from_index(count - 1),
        },
        catches,
        true,
    ));
    if prefix {
        blocks.push(block(throw, FlowEnd::Throw, outer_coverage, true));
    }
    FlowGraph {
        entry: BlockId::from_index(entry),
        blocks: blocks.into(),
    }
}

#[test]
fn harmless_catch_prefixes_do_not_change_shared_handler_nesting() {
    let mut statistics = Vec::new();
    for prefix in [false, true] {
        let graph = shared_catches(prefix, 12);
        let live: IdVec<BlockId, _> = vec![true; graph.blocks.len()].into();
        let ownership = region_ownership(&graph, &live);
        assert_eq!(
            catch_outer_coverage(&graph, &ownership, B13, &graph.blocks[B0].handlers),
            1
        );
        let region = candidate(&graph, &live, &ownership, B0, 0).unwrap();
        assert!(
            region.outer.is_empty(),
            "shared catch must retain its own coverage"
        );
        assert!(region.shared_catches);
        // Installing one fragment makes its catch entries normal logical
        // continuations. That must not make the remaining catches exclusive.
        let mut after = graph.clone();
        crate::structure::installation::install(
            &mut after,
            &mut BTreeMap::new(),
            crate::structure::installation::Region {
                entry: B0,
                nodes: region.nodes,
                outer: region.outer,
                end: region.end,
                body: crate::structure::installation::Body::Unexpanded(region.body),
                may_throw: true,
            },
            None,
        );
        let ownership = region_ownership(&after, &live);
        let next = candidate(&after, &live, &ownership, B1, 0).unwrap();
        assert!(next.shared_catches);
        assert!(next.outer.is_empty());
        let body = recover(&graph).unwrap();
        let stats = crate::structure::statistics(&body);
        assert_eq!(stats.dispatch_regions, 0);
        statistics.push(stats.labeled_regions);
    }
    assert_eq!(statistics[0], statistics[1]);
}

#[test]
fn an_exclusively_owned_inner_catch_retains_its_outer_scope() {
    for prefix in [false, true] {
        let mut graph = shared_catches(prefix, 1);
        graph.entry = B0;
        graph.blocks[B4].end = FlowEnd::Unreachable;
        graph.blocks[B4].handlers.clear();
        let live: IdVec<BlockId, _> = graph.blocks.ids().map(|id| id != B4).collect();
        let ownership = region_ownership(&graph, &live);
        let region = candidate(&graph, &live, &ownership, B0, 0).unwrap();
        assert_eq!(region.outer.len(), 1);
        assert!(!region.shared_catches);
        assert_eq!(
            crate::structure::statistics(&recover(&graph).unwrap()).dispatch_regions,
            0
        );
    }
}

#[test]
fn a_synthetic_dominating_header_precedes_its_numbered_fragments() {
    let handler = HandlerEdge {
        target: B4,
        catch_type: Some("java/io/IOException".into()),
        table_index: 0,
    };
    let original = FlowGraph {
        entry: B0,
        blocks: [
            FlowEnd::Branch {
                then_target: B1,
                else_target: B2,
            },
            FlowEnd::Goto(B3),
            FlowEnd::Goto(B3),
            FlowEnd::Return,
            FlowEnd::Return,
        ]
        .into_iter()
        .enumerate()
        .map(|(id, end)| FlowBlock {
            pc: id as u32,
            end,
            handlers: if id < 3 {
                vec![handler.clone()]
            } else {
                vec![]
            },
            may_throw: id == 1 || id == 2,
            statements_empty: id == 0,
            duplication_cost: None,
            is_loop_update: false,
        })
        .collect(),
    };
    for mapping in [[0, 1, 2, 3, 4], [4, 0, 1, 2, 3]] {
        let mapping: IdVec<BlockId, _> = mapping.into_iter().map(BlockId::from_index).collect();
        for preserve_header_coverage in [false, true] {
            let mut graph = original.clone();
            graph.entry = mapping[original.entry];
            for (id, block) in original.blocks.iter_enumerated() {
                let mut block = block.clone();
                block.end.map_destinations(|target| mapping[target]);
                for handler in &mut block.handlers {
                    handler.target = mapping[handler.target];
                }
                graph.blocks[mapping[id]] = block;
            }
            if !preserve_header_coverage {
                graph.blocks[graph.entry].handlers.clear();
            }
            let body = recover(&graph).unwrap();
            assert!(
                matches!(body.as_slice(), [Stmt::Try { catches, .. }] if catches.len() == 1),
                "{body:#?}"
            );
        }
    }
}

#[test]
fn shadowed_table_suffixes_still_prove_enclosing_catch_scopes() {
    for (inner_type, throwing_catch) in [
        (Some("java/lang/Exception"), true),
        (None, true),
        (Some("java/lang/Exception"), false),
        (None, false),
    ] {
        let outer = HandlerEdge {
            target: B5,
            catch_type: Some("java/lang/Exception".into()),
            table_index: 1,
        };
        let inner = HandlerEdge {
            target: B4,
            catch_type: inner_type.map(String::from),
            table_index: 0,
        };
        let graph = FlowGraph {
            entry: B0,
            blocks: [
                (FlowEnd::Goto(B1), vec![outer.clone()]),
                (FlowEnd::Goto(B2), vec![inner, outer.clone()]),
                (FlowEnd::Goto(B3), vec![outer.clone()]),
                (FlowEnd::Return, vec![]),
                (
                    FlowEnd::Goto(B2),
                    if throwing_catch { vec![outer] } else { vec![] },
                ),
                (FlowEnd::Return, vec![]),
            ]
            .into_iter()
            .enumerate()
            .map(|(id, (end, handlers))| FlowBlock {
                pc: id as u32,
                end,
                handlers,
                may_throw: !matches!(id, 3 | 5) && (id != 4 || throwing_catch),
                statements_empty: false,
                duplication_cost: None,
                is_loop_update: false,
            })
            .collect(),
        };
        let body = recover(&graph).unwrap();
        let catches: Vec<_> = crate::structure::tree::walk(&body)
            .filter_map(|statement| {
                if let Stmt::Try { catches, .. } = statement {
                    Some(catches)
                } else {
                    None
                }
            })
            .flatten()
            .map(|catch| catch.handler)
            .collect();
        assert_eq!(catches, [BlockId::from_index(5), B4], "{body:#?}");
        assert_eq!(crate::structure::statistics(&body).dispatch_regions, 0);
    }
}
