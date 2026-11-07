//! A structural fallback for reducible graphs. Forward jumps become lexical
//! exits; backedges become loop continues. It requires no runtime state.

use super::analysis::Analysis;
use super::attempt::{RecoveryRejection, RecoveryResult, RejectionReason};
use super::{BlockId, FlowEnd, FlowGraph, RegionId, Stmt, SwitchCase};
use crate::index::IdVec;
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
mod scope_tests;
#[cfg(test)]
mod switch_tests;

#[derive(Clone, Copy)]
enum Item {
    Block(BlockId),
    Loop(usize),
}

#[derive(Clone, Copy)]
struct LoopScope {
    header: BlockId,
    region: RegionId,
    follow: Option<BlockId>,
}

struct Builder<'a> {
    graph: &'a FlowGraph,
    analysis: &'a Analysis,
    next_region: usize,
    scopes: Vec<LoopScope>,
    labels: BTreeMap<BlockId, RegionId>,
}

pub(super) fn stackify(graph: &FlowGraph, analysis: &Analysis) -> RecoveryResult<Vec<Stmt>> {
    if let Some((id, _)) = graph.blocks.iter_enumerated().find(|&(id, block)| {
        analysis.reachable[id] && block.may_throw && !block.handlers.is_empty()
    }) {
        return Err(RecoveryRejection::at(id, RejectionReason::ProtectedBlock));
    }
    let mut builder = Builder {
        graph,
        analysis,
        next_region: 0,
        scopes: Vec::new(),
        labels: BTreeMap::new(),
    };
    let body = builder.level(None)?;
    Ok(simplify(body))
}

impl Builder<'_> {
    fn region(&mut self) -> RegionId {
        let id = self.next_region;
        self.next_region += 1;
        id
    }

    fn level(&mut self, parent: Option<usize>) -> RecoveryResult<Vec<Stmt>> {
        let n = self.graph.blocks.len();
        let mut item_for: IdVec<BlockId, _> = vec![usize::MAX; n].into();
        let mut items = Vec::new();
        for (index, lp) in self.analysis.loops.iter().enumerate() {
            if lp.parent == parent {
                let item = items.len();
                for &node in &lp.nodes {
                    item_for[node] = item;
                }
                items.push(Item::Loop(index));
            }
        }
        for (node, slot) in item_for.iter_enumerated_mut() {
            if !self.analysis.reachable[node]
                || parent.is_some_and(|index| !self.analysis.loops[index].contains(node))
            {
                continue;
            }
            if *slot == usize::MAX {
                *slot = items.len();
                items.push(Item::Block(node));
            }
        }
        let count = items.len();
        let mut edges = vec![Vec::new(); count];
        let parent_header = parent.map(|index| self.analysis.loops[index].header);
        for (node, &source) in item_for.iter_enumerated() {
            if source == usize::MAX {
                continue;
            }
            for &target in &self.analysis.successors[node] {
                if Some(target) == parent_header {
                    continue;
                }
                let destination = item_for[target];
                if destination != usize::MAX && destination != source {
                    edges[source].push(destination);
                }
                // A direct self-edge outside a recognized loop is irreducible.
                if destination == source && matches!(items[source], Item::Block(_)) {
                    return Err(RecoveryRejection::at(node, RejectionReason::CyclicOrder));
                }
            }
        }
        let mut indegree = vec![0; count];
        for targets in &mut edges {
            targets.sort_unstable();
            targets.dedup();
            for &target in targets.iter() {
                indegree[target] += 1;
            }
        }
        let head_of = |item: Item| match item {
            Item::Block(block) => block,
            Item::Loop(index) => self.analysis.loops[index].header,
        };
        let mut ready = BTreeSet::new();
        for (index, &degree) in indegree.iter().enumerate() {
            if degree == 0 {
                let head = head_of(items[index]);
                ready.insert((self.graph.blocks[head].pc, head, index));
            }
        }
        let mut order = Vec::with_capacity(count);
        while let Some((_, _, item)) = ready.pop_first() {
            order.push(item);
            for &target in &edges[item] {
                indegree[target] -= 1;
                if indegree[target] == 0 {
                    let head = head_of(items[target]);
                    ready.insert((self.graph.blocks[head].pc, head, target));
                }
            }
        }
        if order.len() != count {
            let blocked = indegree.iter().position(|&degree| degree != 0).unwrap();
            return Err(RecoveryRejection::at(
                head_of(items[blocked]),
                RejectionReason::CyclicOrder,
            ));
        }
        let expected_entry = parent_header.unwrap_or(self.graph.entry);
        if order
            .first()
            .is_some_and(|&item| head_of(items[item]) != expected_entry)
        {
            return Err(RecoveryRejection::at(
                expected_entry,
                RejectionReason::RegionBoundary,
            ));
        }
        let mut position = vec![0; count];
        for (index, &item) in order.iter().enumerate() {
            position[item] = index;
        }
        // A forward exit needs a scope from its earliest source to its target,
        // not from the beginning of the method. Wrapping the whole prefix at
        // every target gives a sequence of unrelated branches linear nesting
        // depth (and quadratic recursive work in the source writer).
        let mut starts = vec![None; count];
        for (source, targets) in edges.iter().enumerate() {
            for &target in targets {
                if position[target] != position[source] + 1 {
                    let start = &mut starts[position[target]];
                    *start = Some(
                        start.map_or(position[source], |old: usize| old.min(position[source])),
                    );
                }
            }
        }
        // Lexical scopes must be disjoint or nested. When two intervals cross,
        // extend the later-ending one to contain the earlier one. The stack
        // holds the maximal preceding intervals; each is visited at most twice.
        let mut intervals: Vec<(usize, usize)> = Vec::new();
        for (end, start) in starts.iter_mut().enumerate() {
            let Some(mut begin) = *start else { continue };
            while intervals
                .last()
                .is_some_and(|&(_, prior_end)| prior_end > begin)
            {
                begin = begin.min(intervals.pop().unwrap().0);
            }
            *start = Some(begin);
            intervals.push((begin, end));
        }
        let old_labels = self.labels.clone();
        let mut opening = vec![Vec::new(); count];
        for (end, start) in starts.into_iter().enumerate() {
            if let Some(start) = start {
                let region = self.region();
                self.labels.insert(head_of(items[order[end]]), region);
                opening[start].push((end, region));
            }
        }
        let mut body = Vec::new();
        let mut scopes: Vec<(usize, RegionId, Vec<Stmt>)> = Vec::new();
        for (position, &item_index) in order.iter().enumerate() {
            let item = items[item_index];
            let head = head_of(item);
            if scopes.last().is_some_and(|&(end, _, _)| end == position) {
                let (_, region, mut outer) = scopes.pop().unwrap();
                outer.push(Stmt::Label { region, body });
                body = outer;
            }
            for &(end, region) in opening[position].iter().rev() {
                scopes.push((end, region, std::mem::take(&mut body)));
            }
            let next = order.get(position + 1).map(|&index| head_of(items[index]));
            match item {
                Item::Loop(index) => {
                    let region = self.region();
                    let follow = match self.analysis.loops[index].exits.as_slice() {
                        [follow] => Some(*follow),
                        _ => next,
                    };
                    self.scopes.push(LoopScope {
                        header: head,
                        region,
                        follow,
                    });
                    let mut inner = self.level(Some(index))?;
                    self.scopes.pop();
                    if matches!(inner.last(), Some(Stmt::Continue { header }) if *header == head) {
                        inner.pop();
                    }
                    body.push(Stmt::Loop {
                        region,
                        header: head,
                        continuation: None,
                        body: inner,
                    });
                    if let Some(follow) = follow {
                        body.extend(self.transfer(follow, next)?);
                    }
                }
                Item::Block(block) => self.block(block, next, &mut body)?,
            }
        }
        debug_assert!(scopes.is_empty());
        self.labels = old_labels;
        Ok(body)
    }

    fn transfer(&self, target: BlockId, next: Option<BlockId>) -> RecoveryResult<Vec<Stmt>> {
        if Some(target) == next {
            return Ok(Vec::new());
        }
        for scope in self.scopes.iter().rev() {
            if target == scope.header {
                return Ok(vec![Stmt::Continue { header: target }]);
            }
        }
        if let Some(&region) = self.labels.get(&target) {
            return Ok(vec![Stmt::Break { target: region }]);
        }
        for scope in self.scopes.iter().rev() {
            if Some(target) == scope.follow {
                return Ok(vec![Stmt::Break {
                    target: scope.region,
                }]);
            }
        }
        Err(RecoveryRejection::at(
            target,
            RejectionReason::RegionBoundary,
        ))
    }

    fn block(
        &mut self,
        block: BlockId,
        next: Option<BlockId>,
        body: &mut Vec<Stmt>,
    ) -> RecoveryResult<()> {
        if let FlowEnd::Multi(exits) = &self.graph.blocks[block].end {
            let exits: RecoveryResult<Vec<_>> = exits
                .iter()
                .map(|&(logical, target)| {
                    self.transfer(target, next)
                        .map(|transfer| (logical, transfer))
                })
                .collect();
            body.push(Stmt::BoundRegion {
                head: block,
                exits: exits?,
            });
            return Ok(());
        }
        body.push(Stmt::Basic(block));
        match &self.graph.blocks[block].end {
            FlowEnd::Goto(target) => body.extend(self.transfer(*target, next)?),
            FlowEnd::Return => body.push(Stmt::Return(block)),
            FlowEnd::Throw => body.push(Stmt::Throw(block)),
            FlowEnd::Unreachable => body.push(Stmt::Unreachable(block)),
            FlowEnd::Multi(_) => unreachable!(),
            FlowEnd::Branch {
                then_target,
                else_target,
            } => {
                let then_body = self.transfer(*then_target, next)?;
                let else_body = self.transfer(*else_target, next)?;
                body.push(super::predicates::fold_if(
                    super::Predicate::Atom {
                        head: block,
                        negated: false,
                    },
                    then_body,
                    else_body,
                ));
            }
            FlowEnd::Switch { cases, default } => {
                let region = self.region();
                let mut grouped = BTreeMap::new();
                for &(key, target) in cases {
                    grouped
                        .entry(target)
                        .or_insert_with(|| SwitchCase {
                            keys: Vec::new(),
                            default: false,
                            body: Vec::new(),
                        })
                        .keys
                        .push(key);
                }
                grouped
                    .entry(*default)
                    .or_insert_with(|| SwitchCase {
                        keys: Vec::new(),
                        default: false,
                        body: Vec::new(),
                    })
                    .default = true;
                let mut cases = Vec::new();
                for (target, mut case) in grouped {
                    case.body = self.transfer(target, next)?;
                    if case.body.is_empty() {
                        case.body.push(Stmt::Break { target: region });
                    }
                    cases.push(case);
                }
                body.push(Stmt::Switch {
                    region,
                    head: block,
                    cases,
                });
            }
        }
        Ok(())
    }
}

pub(super) fn simplify(body: Vec<Stmt>) -> Vec<Stmt> {
    let mut result = Vec::new();
    for stmt in body {
        match stmt.map_bodies(simplify) {
            Stmt::Label { region, body } => result.extend(simplify_label(region, body)),
            other => result.push(other),
        }
    }
    result
}

/// Reduce one label after its children have already been normalized. Keeping
/// this separate from `simplify` avoids revisiting the full descendant tree at
/// every enclosing label in a shared-handler continuation ladder.
pub(super) fn simplify_label(region: RegionId, mut body: Vec<Stmt>) -> Vec<Stmt> {
    if matches!(body.last(), Some(Stmt::Break { target }) if *target == region) {
        body.pop();
    }
    if !super::contains_break(&body, region) {
        return body;
    }
    if let Some(plan) = continuation_plan(&body, &[(region, None)], 0, &[]) {
        return thread_continuation(body, plan, std::collections::VecDeque::new());
    }
    vec![Stmt::Label { region, body }]
}

/// A lexical destination is either normal completion or a surviving break.
/// The same substitution owns branch tails, switch fallthrough, and protected
/// scope exits. Planning borrows the tree; committing moves each suffix once.
type Destinations = [(RegionId, Option<RegionId>)];

enum ContinuationStep {
    Keep,
    Exit(Option<RegionId>),
    Children {
        plans: Vec<Vec<ContinuationStep>>,
        tail: Option<usize>,
        flatten: bool,
    },
    Switch {
        tail_case: Option<usize>,
        plans: Vec<Vec<ContinuationStep>>,
    },
}

fn continuation_plan<'a>(
    body: &'a [Stmt],
    destinations: &Destinations,
    depth: usize,
    inherited_tail: &[&'a Stmt],
) -> Option<Vec<ContinuationStep>> {
    // Retain iterative labels instead of creating unbounded conditional depth.
    if depth > 128 {
        return None;
    }
    let mut plan = Vec::with_capacity(body.len());
    let mut tail = inherited_tail.to_vec();
    for statement in body.iter().rev() {
        if let Stmt::Break { target } = statement
            && let Some(&(_, destination)) = destinations.iter().rev().find(|(r, _)| r == target)
        {
            if destination.is_none() && !tail.is_empty() {
                return None;
            }
            plan.push(ContinuationStep::Exit(destination));
            if destination.is_some() {
                tail.push(statement);
            }
            continue;
        }
        let active: Vec<_> = destinations
            .iter()
            .copied()
            .filter(|&(region, _)| references_region(statement, region))
            .collect();
        if active.is_empty() {
            plan.push(ContinuationStep::Keep);
            tail.push(statement);
            continue;
        }
        // Dispatcher exits are not body statements. A new break also cannot
        // cross a binder that would capture its destination's reused identity.
        if matches!(statement, Stmt::Dispatch { .. })
            || statement.binder().is_some_and(|binder| {
                active
                    .iter()
                    .any(|&(_, destination)| destination == Some(binder.region()))
            })
        {
            return None;
        }
        let completes = active.iter().any(|(_, destination)| destination.is_none());
        let mut nested = active.clone();
        let mut tail_child = None;
        let mut flatten = false;
        if completes {
            if let Stmt::Switch { region, cases, .. } = statement {
                if tail
                    .iter()
                    .any(|statement| references_region(statement, *region))
                {
                    return None;
                }
                // The surviving switch owns outward exits. Its old completion
                // becomes fallthrough to the one case that owns the suffix.
                for (_, destination) in &mut nested {
                    if destination.is_none() {
                        *destination = Some(*region);
                    }
                }
                let tail_case = if tail.is_empty() {
                    None
                } else {
                    if !cases.iter().any(|case| case.default) {
                        return None;
                    }
                    let mut owner = None;
                    for (index, case) in cases.iter().enumerate() {
                        let breaks = super::contains_break(&case.body, *region);
                        let falls = super::can_complete(&case.body);
                        let last = index + 1 == cases.len();
                        if breaks || last && falls {
                            if let Some(previous) = owner {
                                // Several entries may name the same completion.
                                // Earlier cases fall through an empty corridor,
                                // so the suffix still has one source occurrence.
                                if cases[previous + 1..=index].iter().any(|case| {
                                    !case.body.is_empty()
                                        && !matches!(case.body.as_slice(), [Stmt::Break { target }] if target == region)
                                }) {
                                    return None;
                                }
                            }
                            if !last && falls
                                && !cases[index + 1..].iter().any(|case| {
                                    matches!(case.body.as_slice(), [Stmt::Break { target }] if target == region)
                                })
                            {
                                return None;
                            }
                            if super::tree::walk(&case.body).any(|statement| {
                                matches!(statement, Stmt::TryFinally { body, cleanup, .. }
                                    if !super::can_complete(cleanup) && super::contains_break(body, *region))
                            }) {
                                return None;
                            }
                            owner = Some(index);
                        }
                    }
                    Some(owner?)
                };
                let plans = cases
                    .iter()
                    .enumerate()
                    .map(|(index, case)| {
                        let mut destinations = nested.clone();
                        if tail_case.is_some_and(|owner| index <= owner) {
                            destinations.push((*region, None));
                        }
                        continuation_plan(&case.body, &destinations, depth + 1, &[])
                    })
                    .collect::<Option<Vec<_>>>()?;
                plan.push(ContinuationStep::Switch { tail_case, plans });
                tail.push(statement);
                continue;
            }
            match statement {
                Stmt::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    if !tail.is_empty() {
                        tail_child = match (
                            super::can_complete(then_body),
                            super::can_complete(else_body),
                        ) {
                            (true, false) => Some(0),
                            (false, true) => Some(1),
                            _ => return None,
                        };
                    }
                }
                Stmt::Label { region, .. } if tail.is_empty() => {
                    nested.push((*region, None));
                    flatten = true;
                }
                Stmt::Try { .. } | Stmt::Synchronized { .. } if tail.is_empty() => (),
                Stmt::TryFinally { cleanup, .. }
                    if tail.is_empty()
                        && !active.iter().any(|&(region, destination)| {
                            destination.is_none() && super::contains_break(cleanup, region)
                        }) => {}
                _ => return None,
            }
        }
        let plans = statement
            .bodies()
            .enumerate()
            .map(|(index, body)| {
                continuation_plan(
                    body,
                    &nested,
                    depth + 1,
                    if tail_child == Some(index) {
                        &tail
                    } else {
                        &[]
                    },
                )
            })
            .collect::<Option<Vec<_>>>()?;
        plan.push(ContinuationStep::Children {
            plans,
            tail: tail_child,
            flatten,
        });
        tail.push(statement);
    }
    Some(plan)
}

/// Find free transfer references using the same lexical shadowing as binders.
fn references_region(statement: &Stmt, region: RegionId) -> bool {
    let mut pending = vec![statement];
    while let Some(statement) = pending.pop() {
        match statement {
            Stmt::Break { target } | Stmt::DispatchContinue { region: target, .. }
                if *target == region =>
            {
                return true;
            }
            Stmt::Dispatch { exits, .. } if exits.iter().any(|&(_, target)| target == region) => {
                return true;
            }
            _ => (),
        }
        if statement
            .binder()
            .is_none_or(|binder| binder.region() != region)
        {
            pending.extend(statement.bodies().flatten());
        }
    }
    false
}

fn thread_continuation(
    body: Vec<Stmt>,
    plan: Vec<ContinuationStep>,
    mut tail: std::collections::VecDeque<Stmt>,
) -> Vec<Stmt> {
    debug_assert_eq!(body.len(), plan.len());
    for (mut statement, step) in body.into_iter().rev().zip(plan) {
        match step {
            ContinuationStep::Keep => tail.push_front(statement),
            ContinuationStep::Exit(Some(target)) => tail.push_front(Stmt::Break { target }),
            ContinuationStep::Exit(None) => debug_assert!(tail.is_empty()),
            ContinuationStep::Children {
                plans,
                tail: child,
                flatten,
            } => {
                for (index, (body, plan)) in statement.bodies_mut().zip(plans).enumerate() {
                    let suffix = if child == Some(index) {
                        std::mem::take(&mut tail)
                    } else {
                        Default::default()
                    };
                    *body = thread_continuation(std::mem::take(body), plan, suffix);
                }
                if flatten {
                    debug_assert!(tail.is_empty());
                    let Stmt::Label { body, .. } = statement else {
                        unreachable!()
                    };
                    tail.extend(body);
                } else {
                    if let Stmt::If {
                        condition,
                        then_body,
                        else_body,
                    } = statement
                    {
                        statement = super::predicates::fold_if(condition, then_body, else_body);
                    }
                    tail.push_front(statement);
                }
            }
            ContinuationStep::Switch { tail_case, plans } => {
                let Stmt::Switch { region, cases, .. } = &mut statement else {
                    unreachable!()
                };
                for (case, plan) in cases.iter_mut().zip(plans) {
                    case.body = thread_continuation(
                        std::mem::take(&mut case.body),
                        plan,
                        Default::default(),
                    );
                }
                if let Some(index) = tail_case {
                    cases[index].body.extend(std::mem::take(&mut tail));
                    if super::can_complete(&cases[index].body) {
                        cases[index].body.push(Stmt::Break { target: *region });
                    }
                } else {
                    debug_assert!(tail.is_empty());
                }
                tail.push_front(statement);
            }
        }
    }
    tail.into()
}

#[cfg(test)]
mod tests {
    const B0: crate::index::BlockId = crate::index::BlockId::from_index(0);
    const B1: crate::index::BlockId = crate::index::BlockId::from_index(1);
    const B2: crate::index::BlockId = crate::index::BlockId::from_index(2);
    const B3: crate::index::BlockId = crate::index::BlockId::from_index(3);
    const B4: crate::index::BlockId = crate::index::BlockId::from_index(4);
    const B5: crate::index::BlockId = crate::index::BlockId::from_index(5);
    use super::*;
    use crate::structure::{FlowBlock, Predicate};

    #[test]
    fn forward_decisions_share_the_tail_without_repeating_effects() {
        fn decision(head: usize, then_body: Vec<Stmt>) -> Stmt {
            Stmt::If {
                condition: Predicate::Atom {
                    head: BlockId::from_index(head),
                    negated: false,
                },
                then_body,
                else_body: vec![],
            }
        }
        fn execute(body: &[Stmt], bits: u32, trace: &mut Vec<usize>) -> Option<usize> {
            fn test(predicate: &Predicate, bits: u32) -> bool {
                match predicate {
                    Predicate::Atom { head, negated } => {
                        (bits & (1 << head.index()) != 0) != *negated
                    }
                    Predicate::And(terms) => terms.iter().all(|term| test(term, bits)),
                    Predicate::Or(terms) => terms.iter().any(|term| test(term, bits)),
                    Predicate::Select { condition, yes, no } => {
                        test(if test(condition, bits) { yes } else { no }, bits)
                    }
                }
            }
            for statement in body {
                let exit = match statement {
                    Stmt::Basic(block) => {
                        trace.push(block.index());
                        None
                    }
                    Stmt::Break { target } => Some(*target),
                    Stmt::If {
                        condition,
                        then_body,
                        else_body,
                    } => execute(
                        if test(condition, bits) {
                            then_body
                        } else {
                            else_body
                        },
                        bits,
                        trace,
                    ),
                    Stmt::Label { region, body } => {
                        execute(body, bits, trace).filter(|exit| exit != region)
                    }
                    _ => panic!("unexpected statement"),
                };
                if exit.is_some() {
                    return exit;
                }
            }
            None
        }
        let body = vec![
            Stmt::Basic(B0),
            decision(0, vec![Stmt::Basic(B1), Stmt::Break { target: 7 }]),
            Stmt::Basic(B2),
            decision(
                1,
                vec![Stmt::If {
                    condition: Predicate::Atom {
                        head: B2,
                        negated: false,
                    },
                    then_body: vec![Stmt::Basic(B3), Stmt::Break { target: 7 }],
                    else_body: vec![Stmt::Break { target: 7 }],
                }],
            ),
            Stmt::Basic(B4),
            decision(3, vec![Stmt::Break { target: 7 }]),
            Stmt::Basic(B5),
        ];
        let original = vec![Stmt::Label { region: 7, body }];
        let recovered = simplify(original.clone());
        assert!(!super::super::contains_break(&recovered, 7));
        assert!(!matches!(recovered.first(), Some(Stmt::Label { .. })));
        for bits in 0..16 {
            let mut expected = vec![];
            let mut actual = vec![];
            assert_eq!(
                execute(&original, bits, &mut expected),
                execute(&recovered, bits, &mut actual)
            );
            assert_eq!(expected, actual, "branch choices {bits:04b}");
        }
    }

    #[test]
    fn consecutive_forward_regions_do_not_accumulate_lexical_depth() {
        let count = 4096;
        let mut blocks = Vec::with_capacity(count * 2 + 1);
        for index in 0..count {
            for end in [
                FlowEnd::Branch {
                    then_target: BlockId::from_index(index * 2 + 2),
                    else_target: BlockId::from_index(index * 2 + 1),
                },
                FlowEnd::Goto(BlockId::from_index(index * 2 + 2)),
            ] {
                blocks.push(FlowBlock {
                    pc: blocks.len() as u32,
                    statements_empty: false,
                    duplication_cost: None,
                    is_loop_update: false,
                    end,
                    may_throw: false,
                    handlers: Vec::new(),
                });
            }
        }
        blocks.push(FlowBlock {
            pc: blocks.len() as u32,
            statements_empty: false,
            duplication_cost: None,
            is_loop_update: false,
            end: FlowEnd::Return,
            may_throw: false,
            handlers: Vec::new(),
        });
        let graph = FlowGraph {
            entry: B0,
            blocks: blocks.into(),
        };
        let body = stackify(&graph, &Analysis::new(&graph)).unwrap();
        let mut pending = vec![(&body, 0)];
        while let Some((body, depth)) = pending.pop() {
            assert!(depth <= 2, "unrelated branches share a lexical scope");
            for statement in body {
                match statement {
                    Stmt::If {
                        then_body,
                        else_body,
                        ..
                    } => {
                        pending.push((then_body, depth + 1));
                        pending.push((else_body, depth + 1));
                    }
                    Stmt::Label { body, .. } => pending.push((body, depth + 1)),
                    _ => (),
                }
            }
        }
    }
}
