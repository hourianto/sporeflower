use super::*;
const B0: crate::index::BlockId = crate::index::BlockId::from_index(0);
const B1: crate::index::BlockId = crate::index::BlockId::from_index(1);
use crate::structure::{FlowBlock, FlowGraph, analysis::Analysis};

fn graph(ends: Vec<FlowEnd>) -> FlowGraph {
    FlowGraph {
        entry: B0,
        blocks: ends
            .into_iter()
            .enumerate()
            .map(|(id, end)| FlowBlock {
                pc: id as u32,
                statements_empty: matches!(end, FlowEnd::Branch { .. }),
                duplication_cost: None,
                is_loop_update: false,
                end,
                handlers: vec![],
                may_throw: false,
            })
            .collect(),
    }
}

fn retained_heads(planner: &Planner<'_>, result: &Decision) -> usize {
    result.heads.len()
        + planner
            .states
            .iter()
            .map(|state| match state {
                State::Ready(plan) => plan.heads.len(),
                _ => 0,
            })
            .sum::<usize>()
}

#[test]
fn paired_else_if_ladders_plan_each_head_once() {
    // if (a || b) return x; else if (c || d) return y; ...
    // Each pair speculates into the remaining ladder, then tries that same
    // suffix again after combining the pair. It must reuse the failed plan.
    for pairs in [8, 64, 512] {
        let heads = pairs * 2;
        let mut ends = Vec::new();
        for pair in 0..pairs {
            for next in [pair * 2 + 1, pair * 2 + 2] {
                ends.push(FlowEnd::Branch {
                    then_target: BlockId::from_index(heads + pair),
                    else_target: BlockId::from_index(if next == heads {
                        heads + pairs
                    } else {
                        next
                    }),
                });
            }
        }
        ends.extend((0..=pairs).map(|_| FlowEnd::Return));
        let graph = graph(ends);
        let analysis = Analysis::new(&graph);
        let mut builder = Builder::new(&graph, &analysis);
        builder.mark(B0);
        let mut planner = Planner::new(builder);
        let plan = planner.decision(B0, BlockId::from_index(heads), B1);
        assert_eq!(plan.heads, [B0, B1]);
        assert_eq!(
            (plan.yes, plan.no),
            (BlockId::from_index(heads), BlockId::from_index(2))
        );
        let Predicate::Or(terms) = &plan.predicate else {
            panic!("the first pair must remain one short-circuit test")
        };
        assert!(matches!(
            terms.as_slice(),
            [
                Predicate::Atom {
                    head: B0,
                    negated: false
                },
                Predicate::Atom {
                    head: B1,
                    negated: false
                },
            ]
        ));
        assert_eq!(planner.expanded, heads);
        assert_eq!(retained_heads(&planner, &plan), heads);
        let projected = Decisions::project(&graph, &analysis).unwrap();
        assert_eq!(projected.expanded, heads);
        assert_eq!(projected.predicates.iter().flatten().count(), pairs);
    }
}

#[test]
fn adopted_condition_chains_move_instead_of_copying_cached_subtrees() {
    let heads = 2048;
    let mut ends: Vec<_> = (0..heads)
        .map(|head| FlowEnd::Branch {
            then_target: BlockId::from_index(head + 1),
            else_target: BlockId::from_index(heads + 1),
        })
        .collect();
    ends.extend([FlowEnd::Return, FlowEnd::Return]);
    let graph = graph(ends);
    let analysis = Analysis::new(&graph);
    let builder = Builder::new(&graph, &analysis);
    let mut planner = Planner::new(builder);
    let plan = planner.decision(B0, B1, BlockId::from_index(heads + 1));
    assert_eq!(
        (plan.yes, plan.no),
        (BlockId::from_index(heads), BlockId::from_index(heads + 1))
    );
    assert_eq!(
        plan.heads,
        (0..heads).map(BlockId::from_index).collect::<Vec<_>>()
    );
    let Predicate::And(terms) = &plan.predicate else {
        panic!("the complete chain must remain a conjunction")
    };
    assert_eq!(terms.len(), heads);
    assert_eq!(planner.expanded, heads);
    assert_eq!(retained_heads(&planner, &plan), heads);
}

#[test]
fn selected_tests_share_outcomes_without_repeating_the_selector() {
    let b = BlockId::from_index;
    for reversed in [false, true] {
        let input = graph(vec![
            FlowEnd::Branch {
                then_target: b(1),
                else_target: b(2),
            },
            FlowEnd::Branch {
                then_target: b(3),
                else_target: b(4),
            },
            FlowEnd::Branch {
                then_target: b(if reversed { 4 } else { 3 }),
                else_target: b(if reversed { 3 } else { 4 }),
            },
            FlowEnd::Return,
            FlowEnd::Return,
        ]);
        let analysis = Analysis::new(&input);
        let builder = Builder::new(&input, &analysis);
        let mut planner = Planner::new(builder);
        let plan = planner.decision(B0, b(1), b(2));
        assert_eq!((plan.yes, plan.no), (b(3), b(4)));
        assert_eq!(plan.heads, [B0, b(1), b(2)]);
        assert_eq!(planner.expanded, 3);
        assert_eq!(retained_heads(&planner, &plan), 3);
        let Predicate::Select { condition, yes, no } = plan.predicate else {
            panic!("shared outcomes must retain their selecting predicate")
        };
        assert!(matches!(
            *condition,
            Predicate::Atom {
                head: B0,
                negated: false
            }
        ));
        assert!(matches!(*yes, Predicate::Atom { head, negated: false } if head == b(1)));
        assert!(
            matches!(*no, Predicate::Atom { head, negated } if head == b(2) && negated == reversed)
        );

        // A side-effecting statement before either selected test still owns
        // its body; it cannot disappear into a predicate-only region.
        let mut effects = input;
        effects.blocks[b(2)].statements_empty = false;
        let analysis = Analysis::new(&effects);
        let builder = Builder::new(&effects, &analysis);
        let mut planner = Planner::new(builder);
        let plan = planner.decision(B0, b(1), b(2));
        assert!(matches!(plan.predicate, Predicate::Atom { head: B0, .. }));
    }
}
