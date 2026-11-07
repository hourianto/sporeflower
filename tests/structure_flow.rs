use sporeflower::index::{BlockId, IdVec};
const B0: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(0);
const B1: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(1);
const B2: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(2);
const B3: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(3);
const B4: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(4);
const B5: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(5);
const B6: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(6);
const B7: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(7);
const B8: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(8);
const B9: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(9);
const B10: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(10);
const B11: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(11);
const B12: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(12);
const B13: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(13);
const B14: sporeflower::index::BlockId = sporeflower::index::BlockId::from_index(14);
use sporeflower::structure::{FlowBlock, FlowEnd, FlowGraph, Predicate, Stmt, structure_graph};

fn graph(ends: Vec<FlowEnd>) -> FlowGraph {
    FlowGraph {
        entry: B0,
        blocks: ends
            .into_iter()
            .enumerate()
            .map(|(id, end)| FlowBlock {
                pc: id as u32,
                statements_empty: false,
                duplication_cost: None,
                is_loop_update: false,
                may_throw: matches!(end, FlowEnd::Throw),
                end,
                handlers: Vec::new(),
            })
            .collect(),
    }
}

#[test]
fn ordinary_flow_needs_no_allocated_recovery_history() {
    let input = graph(vec![
        FlowEnd::Branch {
            then_target: B1,
            else_target: B2,
        },
        FlowEnd::Return,
        FlowEnd::Return,
    ]);
    let tree = structure_graph(&input).unwrap();
    assert!(tree.recovery_attempts.is_empty());
    assert_eq!(tree.recovery_attempts.capacity(), 0);
    check(&input, false);
}

#[test]
fn unresolved_handler_dispatch_retains_its_recovery_boundary() {
    use sporeflower::structure::{RecoveryRejection, RecoveryStrategy, RejectionReason};
    let mut input = graph(vec![FlowEnd::Throw]);
    input.blocks[B0]
        .handlers
        .push(sporeflower::cfg::HandlerEdge {
            target: B0,
            catch_type: None,
            table_index: 0,
        });
    let tree = structure_graph(&input).unwrap();
    assert_eq!(tree.statistics.dispatch_regions, 1);
    assert_eq!(tree.recovery_attempts.len(), 2);
    assert_eq!(
        tree.recovery_attempts[0].strategy,
        RecoveryStrategy::ExceptionRegions
    );
    assert_eq!(
        tree.recovery_attempts[0].rejection,
        Some(RecoveryRejection {
            block: B0,
            reason: RejectionReason::ProtectedBlock,
        })
    );
    assert_eq!(
        tree.recovery_attempts[1].strategy,
        RecoveryStrategy::Dispatch
    );
    assert_eq!(tree.recovery_attempts[1].rejection, None);
}

#[test]
fn shared_branch_tail_stays_inside_its_switch_case() {
    use FlowEnd::*;
    let graph = graph(vec![
        Switch {
            cases: vec![(0, B1)],
            default: B7,
        },
        Branch {
            then_target: B2,
            else_target: B3,
        },
        Goto(B5),
        Branch {
            then_target: B4,
            else_target: B6,
        },
        Goto(B5),
        Goto(B6),
        Goto(B8),
        Goto(B8),
        Return,
    ]);
    check(&graph, false);
    let tree = structure_graph(&graph).unwrap();
    let Stmt::Switch { cases, .. } = &tree.body[1] else {
        panic!("{tree:#?}")
    };
    assert!(
        cases[0]
            .body
            .iter()
            .any(|statement| matches!(statement, Stmt::Basic(B6)))
    );
    assert!(
        cases[1]
            .body
            .iter()
            .any(|statement| matches!(statement, Stmt::Basic(B7)))
    );
    assert!(matches!(
        tree.body.as_slice(),
        [
            Stmt::Basic(B0),
            Stmt::Switch { .. },
            Stmt::Basic(B8),
            Stmt::Return(B8)
        ]
    ));
}

#[test]
fn nested_case_subsets_own_each_linear_consumer_once_per_path() {
    use FlowEnd::*;
    let mut input = graph(vec![
        Switch {
            cases: vec![(0, B1), (1, B2), (2, B3), (3, B4)],
            default: B9,
        },
        Branch {
            then_target: B5,
            else_target: B9,
        },
        Goto(B5),
        Goto(B6),
        Goto(B7),
        Goto(B7), // load a value for only the first two cases
        Goto(B7),
        Goto(B8), // consume the selected value for all four cases
        Goto(B9),
        Return,
    ]);
    for block in input.blocks.iter_mut().skip(2).take(7) {
        block.duplication_cost = Some(2);
    }
    let tree = structure_graph(&input).unwrap();
    assert_eq!(tree.statistics.labeled_regions, 0, "{tree:#?}");
    check(&input, false);

    // A large shared body keeps its lexical fallback instead of multiplying
    // arbitrary application code across every incoming case.
    input.blocks[B7].duplication_cost = Some(49);
    assert!(structure_graph(&input).unwrap().statistics.labeled_regions > 0);
    check(&input, false);
}

#[test]
fn shared_terminal_subsets_do_not_spill_sibling_switch_cases() {
    use FlowEnd::*;
    let mut input = graph(vec![
        Switch {
            cases: vec![(0, B1), (1, B2), (2, B4), (3, B5)],
            default: B6,
        },
        Branch {
            then_target: B2,
            else_target: B3,
        }, // genuine case fallthrough
        Return,
        Return,
        Branch {
            then_target: B7,
            else_target: B8,
        },
        Branch {
            then_target: B7,
            else_target: B8,
        },
        Goto(B8),
        Throw,
        Return,
    ]);
    input.blocks[B7].duplication_cost = Some(2);
    input.blocks[B8].duplication_cost = Some(1);
    let tree = structure_graph(&input).unwrap();
    assert_eq!(tree.statistics.labeled_regions, 0, "{tree:#?}");
    check(&input, false);
}

#[test]
fn nested_boolean_decisions_expose_the_body_continuation_before_case_recovery() {
    use FlowEnd::*;
    let mut input = graph(vec![
        Switch {
            cases: vec![(0, B1)],
            default: B10,
        },
        Branch {
            then_target: B4,
            else_target: B2,
        },
        Branch {
            then_target: B5,
            else_target: B3,
        },
        Branch {
            then_target: B5,
            else_target: B4,
        },
        Goto(B7),
        Branch {
            then_target: B7,
            else_target: B6,
        },
        Return,
        Branch {
            then_target: B8,
            else_target: B9,
        },
        Goto(B10),
        Goto(B10),
        Return,
    ]);
    for &head in &[B1, B2, B3] {
        input.blocks[head].statements_empty = true;
    }
    let tree = structure_graph(&input).unwrap();
    assert_eq!(tree.statistics.labeled_regions, 0, "{tree:#?}");
    check(&input, false);
}

#[test]
fn shared_case_tails_preserve_loop_update_and_abrupt_exit_traces() {
    use FlowEnd::*;
    let mut input = graph(vec![
        Goto(B1),
        Branch {
            then_target: B2,
            else_target: B10,
        },
        Switch {
            cases: vec![(0, B3), (1, B4), (2, B5)],
            default: B8,
        },
        Branch {
            then_target: B6,
            else_target: B8,
        },
        Goto(B6),
        Goto(B7),
        Goto(B8),
        Return,
        Goto(B9),
        Goto(B1),
        Return,
    ]);
    input.blocks[B6].duplication_cost = Some(4);
    input.blocks[B8].duplication_cost = Some(2);
    input.blocks[B9].is_loop_update = true;
    let tree = structure_graph(&input).unwrap();
    assert_eq!(tree.statistics.labeled_regions, 0, "{tree:#?}");
    check(&input, false);
}

#[test]
fn split_loop_entry_keeps_its_costed_prefix_available_to_branch_arms() {
    use FlowEnd::*;
    let mut input = graph(vec![
        Branch {
            then_target: B1,
            else_target: B4,
        },
        Branch {
            then_target: B2,
            else_target: B3,
        },
        Goto(B6),
        Goto(B5),
        Goto(B5),
        Goto(B6),
        Branch {
            then_target: B7,
            else_target: B8,
        },
        Goto(B5),
        Return,
    ]);
    // The loop can be entered before or after its update. Splitting that
    // entry still leaves two mutually exclusive arms sharing the first update.
    for block in &mut input.blocks {
        if matches!(block.end, Goto(_)) {
            block.duplication_cost = Some(2);
        } else if matches!(block.end, Branch { .. }) {
            block.statements_empty = true;
        }
    }
    input.blocks[B5].is_loop_update = true;
    let tree = structure_graph(&input).unwrap();
    assert_eq!(tree.statistics.labeled_regions, 0, "{tree:#?}");
    check(&input, false);
}

#[test]
fn a_shared_return_after_a_scanning_loop_does_not_stackify_the_outer_branch() {
    use FlowEnd::*;
    // The loop's two exits assign success/failure before B7. Other early
    // returns mean B10 is not a method-wide postdominator of the outer arms.
    let mut input = graph(vec![
        Branch {
            then_target: B1,
            else_target: B9,
        },
        Branch {
            then_target: B2,
            else_target: B10,
        },
        Branch {
            then_target: B3,
            else_target: B6,
        },
        Branch {
            then_target: B4,
            else_target: B5,
        },
        Goto(B7),
        Goto(B2),
        Goto(B7),
        Branch {
            then_target: B8,
            else_target: B10,
        },
        Goto(B10),
        Branch {
            then_target: B10,
            else_target: B11,
        },
        Return,
        Return,
    ]);
    input.blocks[B5].is_loop_update = true;
    let tree = structure_graph(&input).unwrap();
    assert_eq!(tree.statistics.labeled_regions, 0, "{tree:#?}");
    check(&input, false);
}

#[test]
fn switch_continuations_ignore_abrupt_paths_without_requiring_one_loop_latch() {
    use FlowEnd::*;
    let input = graph(vec![
        Goto(B1),
        Branch {
            then_target: B2,
            else_target: B12,
        },
        Switch {
            cases: vec![(0, B3), (1, B4), (2, B5)],
            default: B6,
        },
        Branch {
            then_target: B7,
            else_target: B1,
        },
        Branch {
            then_target: B8,
            else_target: B1,
        },
        Return,
        Goto(B1),
        Goto(B9),
        Goto(B9),
        Branch {
            then_target: B10,
            else_target: B11,
        },
        Goto(B1),
        Goto(B1),
        Return,
    ]);
    let tree = structure_graph(&input).unwrap();
    assert_eq!(tree.statistics.labeled_regions, 0, "{tree:#?}");
    check(&input, false);
}

#[derive(Debug, PartialEq, Eq)]
enum Exit {
    Next,
    Return(BlockId),
    Throw(BlockId),
    Break(usize),
    Continue(BlockId),
    DispatchContinue(usize, BlockId),
    Limit,
}

struct Machine {
    visits: IdVec<BlockId, u32>,
    trace: Vec<BlockId>,
    seed: u32,
}

impl Machine {
    fn new(n: usize, seed: u32) -> Self {
        Self {
            visits: vec![0; n].into(),
            trace: Vec::new(),
            seed,
        }
    }
    fn block(&mut self, id: BlockId) -> bool {
        if self.trace.len() == 1000 {
            return false;
        }
        self.trace.push(id);
        self.visits[id] += 1;
        true
    }
    fn value(&self, id: BlockId) -> i32 {
        ((self
            .seed
            .wrapping_mul(31)
            .wrapping_add(id.index() as u32 * 17)
            .wrapping_add(self.visits[id] * 13))
            % 11) as i32
            - 5
    }
    fn predicate(&self, condition: &Predicate) -> bool {
        match condition {
            Predicate::Atom { head, negated } => (self.value(*head) > 0) != *negated,
            Predicate::And(terms) => terms.iter().all(|term| self.predicate(term)),
            Predicate::Or(terms) => terms.iter().any(|term| self.predicate(term)),
            Predicate::Select { condition, yes, no } => {
                self.predicate(if self.predicate(condition) { yes } else { no })
            }
        }
    }
    fn run_cfg(&mut self, graph: &FlowGraph, mut id: BlockId) -> Exit {
        loop {
            if !graph.blocks[id].statements_empty && !self.block(id) {
                return Exit::Limit;
            }
            id = match &graph.blocks[id].end {
                FlowEnd::Goto(target) => *target,
                FlowEnd::Branch {
                    then_target,
                    else_target,
                } => {
                    if self.value(id) > 0 {
                        *then_target
                    } else {
                        *else_target
                    }
                }
                FlowEnd::Switch { cases, default } => cases
                    .iter()
                    .find(|&&(key, _)| key == self.value(id))
                    .map_or(*default, |&(_, target)| target),
                FlowEnd::Return => return Exit::Return(id),
                FlowEnd::Throw | FlowEnd::Unreachable => return Exit::Throw(id),
                FlowEnd::Multi(_) => panic!("opaque regions are tested through Java execution"),
            };
        }
    }
    fn run_tree(&mut self, graph: &FlowGraph, body: &[Stmt]) -> Exit {
        for stmt in body {
            let exit = match stmt {
                Stmt::Basic(id) | Stmt::Statements { block: id, .. } => {
                    if graph.blocks[*id].statements_empty || self.block(*id) {
                        Exit::Next
                    } else {
                        Exit::Limit
                    }
                }
                Stmt::Return(id) => Exit::Return(*id),
                Stmt::Throw(id) | Stmt::Unreachable(id) => Exit::Throw(*id),
                Stmt::Break { target } => Exit::Break(*target),
                Stmt::Continue { header } => Exit::Continue(*header),
                Stmt::If {
                    condition,
                    then_body,
                    else_body,
                } => self.run_tree(
                    graph,
                    if self.predicate(condition) {
                        then_body
                    } else {
                        else_body
                    },
                ),
                Stmt::Loop {
                    region,
                    header,
                    body,
                    continuation,
                } => loop {
                    match self.run_tree(graph, body) {
                        Exit::Next => (),
                        Exit::Continue(target) if target == *header => (),
                        Exit::Break(target) if target == *region => break Exit::Next,
                        other => break other,
                    }
                    if let Some(block) = continuation
                        && !graph.blocks[*block].statements_empty
                        && !self.block(*block)
                    {
                        break Exit::Limit;
                    }
                },
                Stmt::Label { region, body } => match self.run_tree(graph, body) {
                    Exit::Break(target) if target == *region => Exit::Next,
                    other => other,
                },
                Stmt::Switch {
                    region,
                    head,
                    cases,
                } => {
                    let start = cases
                        .iter()
                        .position(|case| case.keys.contains(&self.value(*head)))
                        .or_else(|| cases.iter().position(|case| case.default));
                    let mut result = Exit::Next;
                    if let Some(start) = start {
                        for case in &cases[start..] {
                            result = self.run_tree(graph, &case.body);
                            if result != Exit::Next {
                                break;
                            }
                        }
                    }
                    if result == Exit::Break(*region) {
                        Exit::Next
                    } else {
                        result
                    }
                }
                Stmt::Try { body, .. } | Stmt::Synchronized { body, .. } => {
                    self.run_tree(graph, body)
                }
                Stmt::TryFinally { body, cleanup, .. } => {
                    let pending = self.run_tree(graph, body);
                    match self.run_tree(graph, cleanup) {
                        Exit::Next => pending,
                        other => other,
                    }
                }
                Stmt::Dispatch {
                    region,
                    entry,
                    cases,
                    exits,
                } => {
                    let mut current = *entry;
                    loop {
                        let Some(case) = cases.iter().find(|case| case.head == current) else {
                            break Exit::Break(
                                exits
                                    .iter()
                                    .find(|(target, _)| *target == current)
                                    .unwrap()
                                    .1,
                            );
                        };
                        match self.run_tree(graph, &case.body) {
                            Exit::Next => (),
                            Exit::DispatchContinue(target, next) if target == *region => {
                                current = next;
                                continue;
                            }
                            other => break other,
                        }
                        current = match &case.end {
                            FlowEnd::Goto(target) => *target,
                            FlowEnd::Branch {
                                then_target,
                                else_target,
                            } => {
                                if self.value(current) > 0 {
                                    *then_target
                                } else {
                                    *else_target
                                }
                            }
                            FlowEnd::Switch { cases, default } => cases
                                .iter()
                                .find(|&&(key, _)| key == self.value(current))
                                .map_or(*default, |&(_, target)| target),
                            FlowEnd::Return => break Exit::Return(current),
                            FlowEnd::Throw | FlowEnd::Unreachable => break Exit::Throw(current),
                            FlowEnd::Multi(_) => {
                                panic!("opaque regions are tested through Java execution")
                            }
                        };
                    }
                }
                Stmt::DispatchContinue { region, target } => {
                    Exit::DispatchContinue(*region, *target)
                }
                Stmt::Jump(_) | Stmt::BoundRegion { .. } => {
                    panic!("unbound or residual region exit in ordinary graph test")
                }
            };
            if exit != Exit::Next {
                return exit;
            }
        }
        Exit::Next
    }
}

fn check(graph: &FlowGraph, allow_dispatch: bool) {
    let structured = structure_graph(graph).unwrap();
    assert!(
        allow_dispatch || structured.statistics.dispatch_regions == 0,
        "unexpected dispatcher: {graph:#?}"
    );
    for seed in 0..23 {
        let mut original = Machine::new(graph.blocks.len(), seed);
        let mut rebuilt = Machine::new(graph.blocks.len(), seed);
        let expected = original.run_cfg(graph, graph.entry);
        let actual = rebuilt.run_tree(graph, &structured.body);
        assert_eq!(
            actual, expected,
            "seed {seed}, graph {graph:#?}, tree {:#?}",
            structured.body
        );
        assert_eq!(
            rebuilt.trace, original.trace,
            "seed {seed}, graph {graph:#?}, tree {:#?}",
            structured.body
        );
    }
}

#[test]
fn returning_switch_arms_share_a_failure_continuation_without_nested_labels() {
    let count = 512;
    let failure = BlockId::from_index(count * 2 + 1);
    let mut ends = vec![FlowEnd::Switch {
        cases: (0..count)
            .map(|index| (index as i32, BlockId::from_index(index * 2 + 1)))
            .collect(),
        default: failure,
    }];
    for index in 0..count {
        ends.push(FlowEnd::Branch {
            then_target: failure,
            else_target: BlockId::from_index(index * 2 + 2),
        });
        ends.push(FlowEnd::Return);
    }
    ends.push(FlowEnd::Throw);
    let graph = graph(ends);
    let result = structure_graph(&graph).unwrap();
    assert_eq!(result.statistics.labeled_regions, 0);
    check(&graph, false);
}

#[test]
fn shared_fallback_keeps_all_incoming_paths() {
    use FlowEnd::*;
    check(
        &graph(vec![
            Branch {
                then_target: B5,
                else_target: B1,
            },
            Branch {
                then_target: B2,
                else_target: B3,
            },
            Goto(B6),
            Switch {
                cases: vec![(4, B4)],
                default: B5,
            },
            Goto(B6),
            Goto(B6),
            Return,
        ]),
        false,
    );
}

#[test]
fn empty_condition_heads_contract_before_shared_arms_are_visited() {
    use FlowEnd::*;
    for conjunction in [false, true] {
        for reversed in [false, true] {
            let mut input = graph(vec![
                Branch {
                    then_target: if conjunction { B1 } else { B2 },
                    else_target: if conjunction { B2 } else { B1 },
                },
                Branch {
                    then_target: if reversed { B2 } else { B3 },
                    else_target: if reversed { B3 } else { B2 },
                },
                Goto(B4),
                Goto(B4),
                Return,
            ]);
            input.blocks[B1].statements_empty = true;
            let tree = structure_graph(&input).unwrap();
            assert_eq!(tree.statistics.labeled_regions, 0, "{tree:#?}");
            assert!(
                tree.body.iter().any(|statement| matches!(
                    statement,
                    Stmt::If {
                        condition: Predicate::And(_) | Predicate::Or(_),
                        ..
                    }
                )),
                "{tree:#?}"
            );
            check(&input, false);
            // An actual statement at the same head must still execute on just
            // its original incoming arm. It cannot become a predicate atom.
            input.blocks[B1].statements_empty = false;
            check(&input, false);
        }
    }
}

#[test]
fn an_empty_condition_with_another_entry_keeps_both_paths() {
    use FlowEnd::*;
    let mut input = graph(vec![
        Branch {
            then_target: B1,
            else_target: B2,
        },
        Branch {
            then_target: B3,
            else_target: B2,
        },
        Branch {
            then_target: B3,
            else_target: B4,
        },
        Goto(B4),
        Return,
    ]);
    input.blocks[B2].statements_empty = true;
    check(&input, false);
}

fn labels_contain_any(body: &[Stmt], blocks: &[usize], inside_label: bool) -> bool {
    body.iter().any(|statement| match statement {
        Stmt::Basic(block) => inside_label && blocks.contains(&block.index()),
        Stmt::Label { body, .. } => labels_contain_any(body, blocks, true),
        Stmt::Loop {
            body, continuation, ..
        } => {
            (inside_label && continuation.is_some_and(|block| blocks.contains(&block.index())))
                || labels_contain_any(body, blocks, inside_label)
        }
        Stmt::If {
            then_body,
            else_body,
            ..
        } => {
            labels_contain_any(then_body, blocks, inside_label)
                || labels_contain_any(else_body, blocks, inside_label)
        }
        Stmt::Switch { cases, .. } => cases
            .iter()
            .any(|case| labels_contain_any(&case.body, blocks, inside_label)),
        _ => false,
    })
}

#[test]
fn shared_switch_tails_keep_surrounding_branches_and_loop_structured() {
    use FlowEnd::*;
    let input = graph(vec![
        Goto(B1),
        Branch {
            then_target: B2,
            else_target: B10,
        },
        Branch {
            then_target: B3,
            else_target: B4,
        },
        Goto(B5),
        Goto(B5),
        Switch {
            cases: vec![(1, B6), (2, B7)],
            default: B8,
        },
        Goto(B9),
        Goto(B9),
        Goto(B11),
        Goto(B11),
        Return,
        Branch {
            then_target: B12,
            else_target: B13,
        },
        Goto(B14),
        Goto(B14),
        Goto(B1),
    ]);
    let tree = structure_graph(&input).unwrap();
    assert!(tree.statistics.labeled_regions > 0, "{tree:#?}");
    assert!(
        !labels_contain_any(&tree.body, &[0, 1, 2, 3, 4, 10, 11, 12, 13, 14], false),
        "{tree:#?}"
    );
    check(&input, false);
}

#[test]
fn an_owned_update_separates_shared_switch_tail_from_continue_cases() {
    use FlowEnd::*;
    let mut input = graph(vec![
        Goto(B1),
        Branch {
            then_target: B2,
            else_target: B9,
        },
        Switch {
            cases: vec![(1, B3), (2, B4), (3, B5)],
            default: B6,
        },
        Goto(B7),
        Goto(B7),
        Goto(B8),
        Branch {
            then_target: B8,
            else_target: B7,
        },
        Goto(B8),
        Goto(B1),
        Return,
    ]);
    input.blocks[B8].is_loop_update = true;
    let tree = structure_graph(&input).unwrap();
    assert_eq!(tree.statistics.labeled_regions, 0, "{tree:#?}");
    assert!(
        tree.body.iter().any(|statement| matches!(
            statement,
            Stmt::Loop {
                continuation: Some(B8),
                ..
            }
        )),
        "{tree:#?}"
    );
    check(&input, false);

    // Copy elimination can make a case enter the continuation directly. It
    // remains a shared follow even though it is also a selector destination.
    input.blocks[B2].end = Switch {
        cases: vec![(1, B7), (2, B4), (3, B5)],
        default: B6,
    };
    let tree = structure_graph(&input).unwrap();
    assert_eq!(tree.statistics.labeled_regions, 0, "{tree:#?}");
    check(&input, false);

    // A direct jump to the guard bypasses the update in the original graph.
    // Such a loop cannot reinterpret continue as its iteration boundary.
    input.blocks[B5].end = Goto(B1);
    let tree = structure_graph(&input).unwrap();
    assert!(
        tree.body.iter().all(|statement| !matches!(
            statement,
            Stmt::Loop {
                continuation: Some(_),
                ..
            }
        )),
        "{tree:#?}"
    );
    check(&input, false);
}

#[test]
fn a_shared_switch_tail_with_outside_entry_is_not_claimed_locally() {
    use FlowEnd::*;
    let input = graph(vec![
        Branch {
            then_target: B1,
            else_target: B5,
        },
        Switch {
            cases: vec![(1, B2), (2, B3)],
            default: B4,
        },
        Goto(B5),
        Goto(B5),
        Goto(B6),
        Goto(B6),
        Return,
    ]);
    let tree = structure_graph(&input).unwrap();
    assert!(labels_contain_any(&tree.body, &[0], false), "{tree:#?}");
    check(&input, false);
}

#[test]
fn loops_keep_multiple_exit_tails_and_outer_continues() {
    use FlowEnd::*;
    check(
        &graph(vec![
            Goto(B1),
            Branch {
                then_target: B8,
                else_target: B2,
            },
            Branch {
                then_target: B6,
                else_target: B3,
            },
            Branch {
                then_target: B5,
                else_target: B4,
            },
            Goto(B3),
            Goto(B1),
            Goto(B7),
            Goto(B9),
            Goto(B9),
            Return,
        ]),
        false,
    );
    check(
        &graph(vec![
            Goto(B1),
            Branch {
                then_target: B6,
                else_target: B2,
            },
            Branch {
                then_target: B1,
                else_target: B3,
            },
            Branch {
                then_target: B5,
                else_target: B4,
            },
            Goto(B2),
            Goto(B1),
            Return,
        ]),
        false,
    );
}

#[test]
fn early_return_and_throw_tails_stay_inside_an_ordinary_loop() {
    use FlowEnd::*;
    for terminal in [Return, Throw] {
        let graph = graph(vec![
            Goto(B1),
            Branch {
                then_target: B2,
                else_target: B5,
            },
            Branch {
                then_target: B3,
                else_target: B4,
            },
            terminal,
            Goto(B1),
            Goto(B6),
            Return,
        ]);
        let result = structure_graph(&graph).unwrap();
        assert_eq!(result.statistics.labeled_regions, 0, "{:#?}", result.body);
        check(&graph, false);
    }
}

#[test]
fn an_owned_exit_corridor_keeps_effects_before_the_shared_follow() {
    use FlowEnd::*;
    let mut input = graph(vec![
        Goto(B1),
        Branch {
            then_target: B2,
            else_target: B5,
        },
        Branch {
            then_target: B3,
            else_target: B4,
        },
        Goto(B5),
        Goto(B1),
        Goto(B6),
        Return,
    ]);
    let tree = structure_graph(&input).unwrap();
    assert_eq!(tree.statistics.labeled_regions, 0, "{tree:#?}");
    check(&input, false);

    // A second entry owns the corridor independently of the loop. Keep that
    // path in the source graph rather than moving its effects into the loop.
    input.blocks[B0].end = Branch {
        then_target: B1,
        else_target: B3,
    };
    check(&input, false);

    // A looping exit path is not an acyclic corridor, even if it eventually
    // reaches the same follow in some executions.
    input.blocks[B0].end = Goto(B1);
    input.blocks[B3].end = Branch {
        then_target: B3,
        else_target: B5,
    };
    check(&input, false);
}

#[test]
fn a_switch_continue_case_does_not_swallow_the_normal_conditional_follow() {
    use FlowEnd::*;
    let mut input = graph(vec![
        Goto(B1),
        Branch {
            then_target: B2,
            else_target: B9,
        },
        Switch {
            cases: vec![(1, B3), (2, B4)],
            default: B8,
        },
        Goto(B5),
        Goto(B5),
        Branch {
            then_target: B6,
            else_target: B8,
        },
        Goto(B7),
        Goto(B8),
        Goto(B1),
        Return,
    ]);
    input.blocks[B8].is_loop_update = true;
    let tree = structure_graph(&input).unwrap();
    assert_eq!(tree.statistics.labeled_regions, 0, "{tree:#?}");
    check(&input, false);

    // The normal case body has another entry outside this switch. Local arm
    // recovery must roll back, preserving the outer branch and its effects.
    input.blocks.push(input.blocks[B2].clone());
    input.blocks[B10].pc = 10;
    input.blocks[B2].end = Branch {
        then_target: B10,
        else_target: B3,
    };
    check(&input, false);
}

#[test]
fn uniquely_owned_terminal_diamond_keeps_its_shared_suffix_once() {
    use FlowEnd::*;
    let graph = graph(vec![
        Goto(B1),
        Branch {
            then_target: B2,
            else_target: B5,
        },
        Branch {
            then_target: B3,
            else_target: B4,
        },
        Branch {
            then_target: B7,
            else_target: B8,
        },
        Goto(B1),
        Goto(B6),
        Return,
        Goto(B9),
        Goto(B9),
        Return,
    ]);
    let result = structure_graph(&graph).unwrap();
    assert_eq!(result.statistics.labeled_regions, 0, "{:#?}", result.body);
    check(&graph, false);
}

#[test]
fn a_terminal_tail_shared_with_an_outside_path_is_not_claimed_by_the_loop() {
    use FlowEnd::*;
    let graph = graph(vec![
        Branch {
            then_target: B1,
            else_target: B5,
        },
        Branch {
            then_target: B2,
            else_target: B6,
        },
        Branch {
            then_target: B5,
            else_target: B3,
        },
        Goto(B1),
        Unreachable,
        Return,
        Return,
    ]);
    check(&graph, false);
}

#[test]
fn random_acyclic_graphs_preserve_execution_traces_without_state() {
    use FlowEnd::*;
    let mut random = 0x31415926_u32;
    let mut next = || {
        random = random.wrapping_mul(1664525).wrapping_add(1013904223);
        random
    };
    for n in 2..35 {
        for _ in 0..12 {
            let mut ends = Vec::new();
            for source in 0..n - 1 {
                let a = BlockId::from_index(source + 1 + next() as usize % (n - source - 1));
                let b = BlockId::from_index(source + 1 + next() as usize % (n - source - 1));
                let c = BlockId::from_index(source + 1 + next() as usize % (n - source - 1));
                ends.push(match next() % 7 {
                    0 => Return,
                    1 => Throw,
                    2 => Goto(a),
                    3 => Switch {
                        cases: vec![(-3, a), (1, b)],
                        default: c,
                    },
                    _ => Branch {
                        then_target: a,
                        else_target: b,
                    },
                });
            }
            ends.push(Return);
            let mut input = graph(ends);
            for block in &mut input.blocks {
                if matches!(block.end, Branch { .. }) {
                    block.statements_empty = next() % 3 == 0;
                }
            }
            check(&input, false);
            for block in &mut input.blocks {
                if matches!(block.end, Goto(_) | Return | Throw) {
                    block.duplication_cost = Some(1);
                }
            }
            check(&input, false);
        }
    }
}

#[test]
fn irreducible_two_entry_cycle_remains_executable() {
    use FlowEnd::*;
    check(
        &graph(vec![
            Branch {
                then_target: B1,
                else_target: B2,
            },
            Branch {
                then_target: B2,
                else_target: B3,
            },
            Branch {
                then_target: B1,
                else_target: B3,
            },
            Return,
        ]),
        true,
    );
}

#[test]
fn ordinary_switch_fallthrough_stays_a_switch_without_forward_labels() {
    use FlowEnd::*;
    let input = graph(vec![
        Switch {
            cases: vec![(1, B1), (2, B2)],
            default: B3,
        },
        Goto(B2),
        Goto(B4),
        Goto(B4),
        Return,
    ]);
    let tree = structure_graph(&input).unwrap();
    assert_eq!(tree.statistics.labeled_regions, 0, "{tree:#?}");
    check(&input, false);
}

#[test]
fn independently_numbered_nested_regions_do_not_complete_an_outer_scope() {
    use sporeflower::structure::{SwitchCase, can_complete};
    let input = graph(vec![FlowEnd::Switch {
        cases: Vec::new(),
        default: B0,
    }]);
    let private_exit = vec![Stmt::Break { target: 0 }];
    for nested in [
        Stmt::Label {
            region: 0,
            body: private_exit.clone(),
        },
        Stmt::Loop {
            region: 0,
            header: B0,
            continuation: None,
            body: private_exit.clone(),
        },
        Stmt::Switch {
            region: 0,
            head: B0,
            cases: vec![SwitchCase {
                keys: Vec::new(),
                default: true,
                body: private_exit,
            }],
        },
    ] {
        let body = vec![Stmt::Label {
            region: 0,
            body: vec![nested, Stmt::Return(B0)],
        }];
        assert_eq!(Machine::new(1, 1).run_tree(&input, &body), Exit::Return(B0));
        assert!(!can_complete(&body), "private inner break: {body:#?}");
    }
    let outward_exit = vec![Stmt::Label {
        region: 0,
        body: vec![Stmt::Label {
            region: 1,
            body: vec![Stmt::Break { target: 0 }],
        }],
    }];
    assert!(can_complete(&outward_exit));
}
