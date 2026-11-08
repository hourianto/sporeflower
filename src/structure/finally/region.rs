//! Cleanup ownership is established before exception handlers become lexical
//! catches or shared continuations. Copies are matched at scheduled statement
//! boundaries; neither a backedge nor a return is itself evidence of cleanup.
use super::{Analysis, completion::Completion, equivalent};
use crate::cfg::HandlerEdge;
use crate::cfg::dispatch::{effective, same_dispatch as same_handlers};
use crate::ir::{MethodIr, Terminator};
use crate::structure::regions::{CleanupCandidate, RegionGraph, VerifiedRegion};
use crate::structure::{
    BlockId, Catch, FlowBlock, FlowEnd, FlowGraph, Predicate, Stmt, SwitchCase,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Point {
    block: BlockId,
    index: usize,
}

struct Template {
    nodes: BTreeSet<BlockId>,
    ends: BTreeSet<Point>,
    body: Vec<Stmt>,
    completion: Completion,
}

struct Copy {
    nodes: BTreeSet<BlockId>,
    end: Point,
    pending_return: Option<BlockId>,
    evidence: CopyEvidence,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CopyEvidence {
    Physical,
    SpecializedNoop,
    SharedContinuation,
}

/// All roots for one handler observe this immutable graph and template. Copy
/// correspondence depends on its scheduled start, not the proposed scope, so
/// both successful and failed matches remain valid until a region is installed.
struct CandidateProof {
    graph: FlowGraph,
    template: Template,
    incoming: crate::index::IdVec<BlockId, Vec<BlockId>>,
    matches: BTreeMap<Point, Option<Rc<Copy>>>,
}

impl CandidateProof {
    fn new(ir: &MethodIr, regions: &RegionGraph, c: &CleanupCandidate) -> Option<Self> {
        let mut graph = regions.flow.clone();
        for block in &mut graph.blocks {
            block.handlers = effective(&block.handlers).cloned().collect();
        }
        let template = template(ir, regions, &graph, c)?;
        let mut incoming: crate::index::IdVec<BlockId, _> =
            vec![Vec::new(); graph.blocks.len()].into();
        for source in crate::structure::analysis::reachable_with_handlers(&graph) {
            let block = &graph.blocks[source];
            let mut targets = block.end.successors();
            if block.may_throw {
                targets.extend(block.handlers.iter().map(|handler| handler.target));
            }
            for target in targets {
                incoming[target].push(source);
            }
        }
        Some(Self {
            graph,
            template,
            incoming,
            matches: BTreeMap::new(),
        })
    }
}

/// Before the cleanup handler, exceptions stay inside the protected body. An
/// inner catch-all can fully handle an operation without dispatching to the
/// enclosing cleanup at all. Its handler paths must then satisfy the same
/// normal-exit proof as the ordinary body paths.
fn protected_handlers(handlers: &[HandlerEdge], cleanup: BlockId) -> Option<&[HandlerEdge]> {
    if let Some(index) = handlers
        .iter()
        .position(|handler| handler.target == cleanup)
    {
        return Some(&handlers[..index]);
    }
    handlers
        .last()
        .is_some_and(|handler| {
            handler
                .catch_type
                .as_deref()
                .is_none_or(|ty| ty == "java/lang/Throwable")
        })
        .then_some(handlers)
}

fn inner<'a>(handlers: &'a [HandlerEdge], outer: &[HandlerEdge]) -> Option<&'a [HandlerEdge]> {
    let split = handlers.len().checked_sub(outer.len())?;
    same_handlers(&handlers[split..], outer).then_some(&handlers[..split])
}

fn slice(ir: &MethodIr, block: BlockId, start: usize, end: usize) -> Vec<Stmt> {
    debug_assert!(end <= ir.blocks[block].statements.len());
    (start != end)
        .then_some(Stmt::Statements { block, start, end })
        .into_iter()
        .collect()
}

/// A previously certified source region is one cleanup operation. Its own
/// catches and releases stay inside it; only its outer exception dispatch and
/// a single fallthrough continuation participate in the enclosing proof.
/// Returns or logical outward transfers cannot be hidden behind this atom.
fn closed_region<'a>(
    regions: &'a RegionGraph,
    graph: &FlowGraph,
    node: BlockId,
) -> Option<(&'a [Stmt], BlockId)> {
    let body = regions.replacements.get(&node)?;
    let FlowEnd::Goto(next) = graph.blocks[node].end else {
        return None;
    };
    (regions
        .normal_completions(node)?
        .represented_by_edges(&graph.blocks[node].end)
        && super::cleanup_control(body))
    .then_some((body, next))
}

/// A monitor region retains the scheduled acquisition prefix at its entry.
/// Match that prefix at statement granularity: a return value can be saved
/// immediately before cleanup begins in the same original block. The owned
/// synchronized body remains one indivisible operation after this prefix.
fn region_parts<'a>(
    ir: &MethodIr,
    regions: &'a RegionGraph,
    node: BlockId,
) -> Option<(usize, &'a [Stmt])> {
    let body = regions.replacements.get(&node)?;
    match body.first() {
        Some(Stmt::Basic(block)) if *block == node => {
            Some((ir.blocks[node].statements.len(), &body[1..]))
        }
        _ => Some((0, body)),
    }
}

fn template(
    ir: &MethodIr,
    regions: &RegionGraph,
    graph: &FlowGraph,
    c: &CleanupCandidate,
) -> Option<Template> {
    let caught = ir.blocks[c.handler].caught_exception?;
    let mut nodes = BTreeSet::new();
    let mut ends = BTreeSet::new();
    let mut pending = vec![c.handler];
    while let Some(node) = pending.pop() {
        if !nodes.insert(node) {
            continue;
        }
        if regions.replacements.contains_key(&node) {
            let (body, next) = closed_region(regions, graph, node)?;
            if equivalent::reads_value(ir, body, caught) {
                return None;
            }
            if graph.blocks[node].may_throw {
                for handler in inner(&graph.blocks[node].handlers, &c.outer)? {
                    if handler.target == c.handler {
                        return None;
                    }
                    pending.push(handler.target);
                }
            }
            pending.push(next);
            continue;
        }
        let data = &ir.blocks[node];
        let rethrow = matches!(data.terminator, Terminator::Throw { value, .. } if value == caught);
        let returning = matches!(data.terminator, Terminator::Return(_));
        let mut read_body = vec![Stmt::Basic(node)];
        if !rethrow {
            match data.terminator {
                Terminator::Throw { .. } => read_body.push(Stmt::Throw(node)),
                Terminator::Branch { .. } => read_body.push(Stmt::If {
                    condition: Predicate::Atom {
                        head: node,
                        negated: false,
                    },
                    then_body: vec![],
                    else_body: vec![],
                }),
                Terminator::Switch { .. } => read_body.push(Stmt::Switch {
                    region: 0,
                    head: node,
                    cases: vec![],
                }),
                Terminator::Return(_) => read_body.push(Stmt::Return(node)),
                Terminator::Unreachable => return None,
                _ => (),
            }
        }
        if equivalent::reads_value(ir, &read_body, caught)
            || data
                .statements
                .iter()
                .any(|statement| matches!(statement, crate::ir::Statement::Monitor { .. }))
        {
            return None;
        }
        if graph.blocks[node].may_throw {
            for handler in inner(&graph.blocks[node].handlers, &c.outer)? {
                if handler.target == c.handler {
                    return None;
                }
                pending.push(handler.target);
            }
        }
        if rethrow || returning {
            ends.insert(Point {
                block: node,
                index: data.statements.len(),
            });
        } else {
            pending.extend(graph.blocks[node].end.successors());
        }
    }
    if ends.is_empty() {
        return None;
    }
    // A bare catch/rethrow has no cleanup witness. In particular, it must not
    // acquire an implicit-finally exception contract merely because its empty
    // body can match any return site.
    if !nodes.iter().any(|&node| {
        regions.replacements.contains_key(&node)
            || !ir.blocks[node].statements.is_empty()
            || matches!(
                ir.blocks[node].terminator,
                Terminator::Branch { .. } | Terminator::Switch { .. }
            )
    }) {
        return None;
    }
    let body = cleanup_body(ir, regions, graph, c, &nodes, &ends, None)?;
    let proof_body = crate::structure::installation::expand(body.clone(), &regions.replacements);
    if !super::cleanup_control(&proof_body) || !crate::structure::can_complete(&proof_body) {
        return None;
    }
    let completion = Completion::certify(
        ir,
        caught,
        ends.iter().map(|point| point.block),
        &proof_body,
    )?;
    Some(Template {
        nodes,
        ends,
        body,
        completion,
    })
}

/// The ownership and copy certificate uses the original graph. Only this
/// source view may discard checked catches which have no possible effect.
fn cleanup_body(
    ir: &MethodIr,
    regions: &RegionGraph,
    graph: &FlowGraph,
    c: &CleanupCandidate,
    nodes: &BTreeSet<BlockId>,
    ends: &BTreeSet<Point>,
    filter: Option<&crate::structure::HandlerFilter<'_>>,
) -> Option<Vec<Stmt>> {
    let boundary = graph.blocks.next_id();
    let mut sub = graph.clone();
    sub.blocks.push(FlowBlock {
        pc: u32::MAX,
        statements_empty: false,
        duplication_cost: None,
        is_loop_update: false,
        end: FlowEnd::Unreachable,
        handlers: vec![],
        may_throw: false,
    });
    let mut allowed: crate::index::IdVec<BlockId, _> = vec![false; sub.blocks.len()].into();
    for &node in nodes {
        allowed[node] = true;
        let block = &mut sub.blocks[node];
        // Pure guards retain the table's lexical scope even though they cannot
        // dispatch. Erasing it fragments one cleanup catch into separate tries.
        block.handlers = inner(&block.handlers, &c.outer)
            .or_else(|| (!block.may_throw).then_some(&[]))?
            .to_vec();
    }
    let mut slices = BTreeMap::new();
    for end in ends {
        sub.blocks[end.block].end = FlowEnd::Goto(boundary);
        sub.blocks[end.block].may_throw = ir.blocks[end.block]
            .statements
            .iter()
            .any(|s| crate::structure::statement_may_throw(ir, s));
        slices.insert(end.block, slice(ir, end.block, 0, end.index));
    }
    let mut effects = regions.replacements.clone();
    effects.extend(slices.iter().map(|(&node, body)| (node, body.clone())));
    let eligibility = crate::structure::eligibility::Eligibility::new(&sub, &effects, filter);
    let body = crate::structure::exception::subgraph_with_exits(
        &sub,
        c.handler,
        &[(boundary, vec![])],
        Some(&allowed),
        &[],
        0,
        false,
        &eligibility,
    )?;
    Some(crate::structure::installation::expand(body, &slices))
}

fn normalized(
    ir: &MethodIr,
    regions: &RegionGraph,
    graph: &FlowGraph,
    mut point: Point,
    stop: &BTreeSet<Point>,
    nodes: &mut BTreeSet<BlockId>,
) -> Option<Point> {
    let mut empty = BTreeSet::new();
    loop {
        nodes.insert(point.block);
        if stop.contains(&point) || point.index != ir.blocks[point.block].statements.len() {
            return Some(point);
        }
        if regions.replacements.contains_key(&point.block) {
            return Some(point);
        }
        let FlowEnd::Goto(next) = graph.blocks[point.block].end else {
            return Some(point);
        };
        if !empty.insert(point.block) {
            return None;
        }
        point = Point {
            block: next,
            index: 0,
        };
    }
}

fn binding(handler: BlockId, catch_type: &Option<String>) -> Stmt {
    Stmt::Try {
        body: vec![],
        catches: vec![Catch {
            handler,
            catch_type: catch_type.clone(),
            body: vec![],
        }],
    }
}

fn same_completion(
    proof: &Analysis<'_>,
    regions: &RegionGraph,
    graph: &FlowGraph,
    mut left: Point,
    mut right: Point,
) -> Option<BTreeSet<BlockId>> {
    let ir = proof.ir;
    let mut left_nodes = BTreeSet::new();
    let mut right_nodes = BTreeSet::new();
    let mut right_writes = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut a = Vec::new();
    let mut b = Vec::new();
    loop {
        left = normalized(ir, regions, graph, left, &BTreeSet::new(), &mut left_nodes)?;
        right = normalized(
            ir,
            regions,
            graph,
            right,
            &BTreeSet::new(),
            &mut right_nodes,
        )?;
        if left == right {
            // The shared continuation stays in the graph. It must not also
            // contain operations already replayed through the selected suffix.
            if right_writes.contains(&right.block) {
                return None;
            }
            right_nodes.remove(&right.block);
            break;
        }
        if !seen.insert((left, right))
            || regions.replacements.contains_key(&left.block)
            || regions.replacements.contains_key(&right.block)
        {
            return None;
        }
        let ad = &ir.blocks[left.block];
        let bd = &ir.blocks[right.block];
        match (
            ad.statements.get(left.index),
            bd.statements.get(right.index),
        ) {
            (Some(x), Some(y)) => {
                if crate::structure::statement_may_throw(ir, x)
                    || crate::structure::statement_may_throw(ir, y)
                {
                    return None;
                }
                a.push(Stmt::Statements {
                    block: left.block,
                    start: left.index,
                    end: left.index + 1,
                });
                b.push(Stmt::Statements {
                    block: right.block,
                    start: right.index,
                    end: right.index + 1,
                });
                right_writes.insert(right.block);
                left.index += 1;
                right.index += 1;
            }
            (None, None) => {
                let (Terminator::Return(x), Terminator::Return(y)) =
                    (&ad.terminator, &bd.terminator)
                else {
                    return None;
                };
                if !x.is_none_or(|value| equivalent::stable_return(ir, value, &[]))
                    || !y.is_none_or(|value| equivalent::stable_return(ir, value, &[]))
                    || !equivalent::same_exit(
                        ir,
                        &Stmt::Return(left.block),
                        &Stmt::Return(right.block),
                    )
                {
                    return None;
                }
                // An equivalent pure return is a retained continuation, not
                // a cleanup operation. It may also serve paths that never
                // entered this protected scope. Only duplicated assignments
                // before the return require exclusive ownership.
                if !right_writes.contains(&right.block) {
                    right_nodes.remove(&right.block);
                }
                break;
            }
            _ => return None,
        }
    }
    // These assignments are preserved in the selected suffix, not treated as
    // transparent. The ordinary ownership check also covers every duplicate
    // suffix node consumed by this correspondence.
    // Empty connectors may occur in both suffixes. They are still part of the
    // selected suffix and must remain available after its retained entry.
    right_nodes.retain(|node| !left_nodes.contains(node));
    proof.same(&a, &b).then_some(right_nodes)
}

/// First establish corresponding graph points, then compare typed operations
/// with the complete set of participating sites. This keeps temporary alpha
/// renaming bounded to the copied cleanup, rather than the whole method.
fn match_copy(
    proof: &Analysis<'_>,
    regions: &RegionGraph,
    graph: &FlowGraph,
    c: &CleanupCandidate,
    template: &Template,
    start: Point,
    specialize: bool,
) -> Option<Copy> {
    let ir = proof.ir;
    // Empty connectors remain part of the protected graph. Cutting before
    // such an edge would create several apparent copies of one shared cleanup
    // reached through different predecessors, obscuring its single entry.
    if start.index == ir.blocks[start.block].statements.len()
        && matches!(graph.blocks[start.block].end, FlowEnd::Goto(_))
        && !regions.replacements.contains_key(&start.block)
    {
        return None;
    }
    let mut pending = vec![(
        Point {
            block: c.handler,
            index: 0,
        },
        start,
    )];
    let mut mapping = BTreeMap::new();
    let mut inverse = BTreeMap::new();
    let mut nodes = BTreeSet::new();
    let mut left_nodes = BTreeSet::new();
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut bindings = BTreeSet::new();
    let mut stable_reads = BTreeSet::new();
    let mut end = None;
    while let Some((mut a, mut b)) = pending.pop() {
        // Only repetition within one effect-free specialization chain proves
        // a zero-progress cycle. Distinct branches can converge on the same
        // specialized null guard; those are ordinary graph joins, deduplicated
        // by the correspondence map after both points have been normalized.
        let mut specialized = BTreeSet::new();
        'normalize: loop {
            a = normalized(ir, regions, graph, a, &template.ends, &mut left_nodes)?;
            if template.ends.contains(&a) {
                break;
            }
            b = normalized(ir, regions, graph, b, &BTreeSet::new(), &mut nodes)?;
            // Normal and exceptional paths may share the complete cleanup
            // after an empty catch prologue. Correspondence must reach the
            // same point; entering an unmatched suffix is still not a copy.
            if template.nodes.contains(&b.block) && (a != b || b.block == c.handler) {
                return None;
            }
            // Query the canonical cleanup's reads at this normal copy's entry,
            // never at the exceptional handler entry. Reintroduced tests must be
            // effect-free and retain those contents throughout the matched copy.
            for canonical in [true, false] {
                let point = if canonical { a } else { b };
                let context = if canonical { start } else { b };
                if specialize
                    && point.index == ir.blocks[point.block].statements.len()
                    && !regions.replacements.contains_key(&point.block)
                    && let Terminator::Branch {
                        condition,
                        then_target,
                        else_target,
                    } = &ir.blocks[point.block].terminator
                    && let Some(decision) = super::specialization::condition(
                        proof,
                        (context.block, context.index),
                        condition,
                    )
                {
                    if !specialized.insert((a, b)) {
                        return None;
                    }
                    let next = Point {
                        block: if decision.taken {
                            *then_target
                        } else {
                            *else_target
                        },
                        index: 0,
                    };
                    if canonical {
                        stable_reads.extend(decision.reads);
                        a = next;
                    } else {
                        b = next;
                    }
                    continue 'normalize;
                }
            }
            break;
        }
        if template.ends.contains(&a) {
            // The copied cleanup has ended at this scheduled position. Gotos
            // after it belong to the continuation, which may also have normal
            // predecessors outside the protected region.
            if let Some(end) = end {
                nodes.extend(same_completion(proof, regions, graph, end, b)?);
            }
            end.get_or_insert(b);
            continue;
        }
        if let Some(previous) = mapping.insert(a, b) {
            if previous != b {
                return None;
            }
            continue;
        }
        if inverse.insert(b, a).is_some_and(|previous| previous != a) {
            return None;
        }
        let a_parts = region_parts(ir, regions, a.block);
        let b_parts = region_parts(ir, regions, b.block);
        let a_macro_end = a_parts.is_some_and(|(prefix, _)| a.index == prefix);
        let b_macro_end = b_parts.is_some_and(|(prefix, _)| b.index == prefix);
        if a_macro_end != b_macro_end {
            return None;
        }
        let ad = &ir.blocks[a.block];
        let bd = &ir.blocks[b.block];
        let throwing = a_macro_end
            || if a.index < ad.statements.len() {
                crate::structure::statement_may_throw(ir, &ad.statements[a.index])
            } else {
                crate::structure::terminator_may_throw(ir, &ad.terminator)
            };
        if throwing {
            let ah = inner(&graph.blocks[a.block].handlers, &c.outer)?;
            let bh = inner(&graph.blocks[b.block].handlers, &c.outer)?;
            if bh.iter().any(|h| h.target == c.handler) || ah.len() != bh.len() {
                return None;
            }
            for (a, b) in ah.iter().zip(bh) {
                if a.catch_type != b.catch_type {
                    return None;
                }
                if bindings.insert((a.target, b.target)) {
                    left.push(binding(a.target, &a.catch_type));
                    right.push(binding(b.target, &b.catch_type));
                    pending.push((
                        Point {
                            block: a.target,
                            index: 0,
                        },
                        Point {
                            block: b.target,
                            index: 0,
                        },
                    ));
                }
            }
        }
        if a_macro_end {
            let (_, a_next) = closed_region(regions, graph, a.block)?;
            let (_, b_next) = closed_region(regions, graph, b.block)?;
            left.extend_from_slice(a_parts?.1);
            right.extend_from_slice(b_parts?.1);
            pending.push((
                Point {
                    block: a_next,
                    index: 0,
                },
                Point {
                    block: b_next,
                    index: 0,
                },
            ));
            continue;
        }
        if a.index < ad.statements.len() {
            if b.index == bd.statements.len() {
                return None;
            }
            left.push(Stmt::Statements {
                block: a.block,
                start: a.index,
                end: a.index + 1,
            });
            right.push(Stmt::Statements {
                block: b.block,
                start: b.index,
                end: b.index + 1,
            });
            pending.push((
                Point {
                    index: a.index + 1,
                    ..a
                },
                Point {
                    index: b.index + 1,
                    ..b
                },
            ));
            continue;
        }
        if b.index != bd.statements.len() {
            return None;
        }
        match (&ad.terminator, &bd.terminator) {
            (
                Terminator::Branch {
                    condition: ac,
                    then_target: at,
                    else_target: af,
                },
                Terminator::Branch {
                    condition: bc,
                    then_target: bt,
                    else_target: bf,
                },
            ) => {
                let inverted = ac.op != bc.op;
                if inverted && ac.op != bc.op.inverted() {
                    return None;
                }
                left.push(Stmt::If {
                    condition: Predicate::Atom {
                        head: a.block,
                        negated: false,
                    },
                    then_body: vec![],
                    else_body: vec![],
                });
                right.push(Stmt::If {
                    condition: Predicate::Atom {
                        head: b.block,
                        negated: inverted,
                    },
                    then_body: vec![],
                    else_body: vec![],
                });
                let (bt, bf) = if inverted { (bf, bt) } else { (bt, bf) };
                pending.push((
                    Point {
                        block: *at,
                        index: 0,
                    },
                    Point {
                        block: *bt,
                        index: 0,
                    },
                ));
                pending.push((
                    Point {
                        block: *af,
                        index: 0,
                    },
                    Point {
                        block: *bf,
                        index: 0,
                    },
                ));
            }
            (
                Terminator::Switch {
                    cases: ac,
                    default: ad,
                    ..
                },
                Terminator::Switch {
                    cases: bc,
                    default: bd,
                    ..
                },
            ) => {
                if ac.len() != bc.len() {
                    return None;
                }
                left.push(Stmt::Switch {
                    region: 0,
                    head: a.block,
                    cases: vec![],
                });
                right.push(Stmt::Switch {
                    region: 0,
                    head: b.block,
                    cases: vec![],
                });
                for ((ak, at), (bk, bt)) in ac.iter().zip(bc) {
                    if ak != bk {
                        return None;
                    }
                    pending.push((
                        Point {
                            block: *at,
                            index: 0,
                        },
                        Point {
                            block: *bt,
                            index: 0,
                        },
                    ));
                }
                pending.push((
                    Point {
                        block: *ad,
                        index: 0,
                    },
                    Point {
                        block: *bd,
                        index: 0,
                    },
                ));
            }
            (Terminator::Throw { .. }, Terminator::Throw { .. }) => {
                left.push(Stmt::Throw(a.block));
                right.push(Stmt::Throw(b.block));
            }
            _ => return None,
        }
    }
    let end = end?;
    // An entirely specialized-away cleanup has no operation witnessing where
    // its copy begins. It may discharge an actual return, but must not cut an
    // arbitrary protected-body prefix merely because its locals are still
    // initialized to values making the cleanup a no-op.
    if left.is_empty()
        && right.is_empty()
        && (end.index != ir.blocks[end.block].statements.len()
            || !matches!(ir.blocks[end.block].terminator, Terminator::Return(_)))
    {
        return None;
    }
    if !stable_reads.is_empty() {
        let mut writes = equivalent::written_locals(ir, &left);
        writes.extend(equivalent::written_locals(ir, &right));
        if !stable_reads.is_disjoint(&writes) {
            return None;
        }
    }
    if !equivalent::same_at(proof, &left, &right, Some((start.block, start.index))) {
        return None;
    }
    let pending_return = if template.completion.returns() {
        Some(super::completion::returning_copy(
            ir,
            graph,
            &regions.replacements,
            end.block,
            end.index,
            &left,
            &right,
        )?)
    } else {
        None
    };
    Some(Copy {
        nodes,
        end,
        pending_return,
        evidence: if mapping
            .values()
            .any(|point| !template.nodes.contains(&point.block))
        {
            CopyEvidence::Physical
        } else if mapping.is_empty() && !template.nodes.contains(&start.block) {
            CopyEvidence::SpecializedNoop
        } else {
            CopyEvidence::SharedContinuation
        },
    })
}

fn suffix(
    ir: &MethodIr,
    regions: &RegionGraph,
    graph: &FlowGraph,
    point: Point,
) -> Option<VerifiedRegion> {
    if let Some((prefix, tail)) = region_parts(ir, regions, point.block) {
        // A copy can end within the acquisition prefix of an already-owned
        // region. Its remaining operations include the semantic atom, not the
        // original bytecode edge into that atom's now-consumed interior.
        if point.index > prefix {
            return None;
        }
        let mut body = slice(ir, point.block, point.index, prefix);
        body.extend_from_slice(tail);
        let block = &graph.blocks[point.block];
        return Some(VerifiedRegion {
            entry: point.block,
            nodes: vec![point.block],
            outer: block.handlers.clone(),
            end: block.end.clone(),
            body,
            may_throw: block.may_throw,
            cleanups: Default::default(),
        });
    }
    let mut body = slice(
        ir,
        point.block,
        point.index,
        ir.blocks[point.block].statements.len(),
    );
    let targets = graph.blocks[point.block].end.successors();
    match &ir.blocks[point.block].terminator {
        Terminator::Goto(target) => body.push(Stmt::Jump(*target)),
        Terminator::Branch {
            then_target,
            else_target,
            ..
        } => body.push(Stmt::If {
            condition: Predicate::Atom {
                head: point.block,
                negated: false,
            },
            then_body: vec![Stmt::Jump(*then_target)],
            else_body: vec![Stmt::Jump(*else_target)],
        }),
        Terminator::Switch { cases, default, .. } => {
            let mut grouped = BTreeMap::<BlockId, Vec<i32>>::new();
            for &(key, target) in cases {
                grouped.entry(target).or_default().push(key);
            }
            grouped.entry(*default).or_default();
            body.push(Stmt::Switch {
                region: 0,
                head: point.block,
                cases: grouped
                    .into_iter()
                    .map(|(target, keys)| SwitchCase {
                        keys,
                        default: target == *default,
                        body: vec![Stmt::Jump(target)],
                    })
                    .collect(),
            });
        }
        Terminator::Return(_) => body.push(Stmt::Return(point.block)),
        Terminator::Throw { .. } => body.push(Stmt::Throw(point.block)),
        Terminator::Unreachable => return None,
    }
    let may_throw = ir.blocks[point.block].statements[point.index..]
        .iter()
        .any(|s| crate::structure::statement_may_throw(ir, s))
        || crate::structure::terminator_may_throw(ir, &ir.blocks[point.block].terminator);
    Some(VerifiedRegion {
        entry: point.block,
        nodes: vec![point.block],
        outer: graph.blocks[point.block].handlers.clone(),
        end: FlowEnd::Multi(targets.into_iter().map(|target| (target, target)).collect()),
        body,
        may_throw,
        cleanups: Default::default(),
    })
}

/// Prefer the complete catch-all scope, including earlier typed handlers and
/// every normal copy. Smaller scopes may borrow the exceptional cleanup as a
/// witness, but cannot retire its nodes or consume its shared rethrow fact.
pub(in crate::structure) fn plan_candidate(
    proof: &Analysis<'_>,
    regions: &RegionGraph,
    c: &CleanupCandidate,
    filter: Option<&crate::structure::HandlerFilter<'_>>,
) -> Option<Vec<VerifiedRegion>> {
    let ir = proof.ir;
    let mut context = CandidateProof::new(ir, regions, c)?;
    if let Some(plan) = plan_scope(proof, regions, &mut context, c, Witness::Owned, filter) {
        return Some(plan);
    }
    // A shared witness can serve a whole catch, a protected scope reached
    // after unprotected code, or an opaque region's continuation. A smaller
    // interior scope must consolidate multiple normal copies; otherwise it
    // merely moves a single copy into an extra try. Include pure branch/catch
    // entries so the largest proved scope precedes its throwing suffixes.
    let mut covered: BTreeSet<_> = regions
        .flow
        .blocks
        .iter_enumerated()
        .filter_map(|(node, block)| {
            block
                .handlers
                .iter()
                .any(|handler| handler.target == c.handler)
                .then_some(node)
        })
        .collect();
    // Preparation can move a pure initialization/catch prefix to a separate
    // block. It retains the same origin as its protected throwing tail and is
    // still part of that scope for choosing an entry.
    for (node, block) in regions.flow.blocks.iter_enumerated() {
        if !block.may_throw
            && let FlowEnd::Goto(tail) = block.end
            && ir.blocks[node].pc == ir.blocks[tail].pc
            && covered.contains(&tail)
        {
            covered.insert(node);
        }
    }
    let entries: Vec<_> = regions
        .flow
        .blocks
        .ids()
        .filter(|entry| *entry == c.entry || covered.contains(entry))
        .collect();
    for entry in entries {
        let boundary = entry == c.entry
            || ir.blocks[entry].caught_exception.is_some()
            || context.incoming[entry].iter().any(|&source| {
                regions.replacements.contains_key(&source) || !covered.contains(&source)
            });
        if let Some(plan) = plan_scope(
            proof,
            regions,
            &mut context,
            c,
            Witness::Shared { entry, boundary },
            filter,
        ) {
            return Some(plan);
        }
    }
    None
}

/// Only a complete scope owns the canonical handler and its rethrow fact.
#[derive(Clone, Copy)]
enum Witness {
    Owned,
    Shared { entry: BlockId, boundary: bool },
}

fn plan_scope(
    proof: &Analysis<'_>,
    regions: &RegionGraph,
    context: &mut CandidateProof,
    c: &CleanupCandidate,
    witness: Witness,
    filter: Option<&crate::structure::HandlerFilter<'_>>,
) -> Option<Vec<VerifiedRegion>> {
    let ir = proof.ir;
    let CandidateProof {
        graph,
        template,
        incoming,
        matches,
    } = context;
    let (root, shared, boundary) = match witness {
        Witness::Owned => (c.entry, false, true),
        Witness::Shared { entry, boundary } => (entry, true, boundary),
    };
    let mut body_nodes = BTreeSet::new();
    let mut copies = BTreeMap::<BlockId, (usize, Rc<Copy>)>::new();
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if !body_nodes.insert(node) {
            continue;
        }
        if node == c.handler {
            return None;
        }
        let block = &graph.blocks[node];
        let protected = protected_handlers(&block.handlers, c.handler);
        if let Some(completions) = regions.normal_completions(node) {
            // A logical graph exit can be matched with its normal cleanup.
            // A return already enclosed in an opaque child has no such edge;
            // enclosing it in finally would silently add cleanup to that path.
            if !completions.represented_by_edges(&block.end) {
                return None;
            }
        }
        let mut cut = None;
        let end = region_parts(ir, regions, node)
            .map_or(ir.blocks[node].statements.len(), |(prefix, _)| prefix);
        for index in 0..=end {
            // Matching identical control first preserves cleanup-local writes
            // across loop iterations. Specializing a known entry guard can be
            // invalid once that cleanup updates its own tested local.
            let start = Point { block: node, index };
            let matched = matches.entry(start).or_insert_with(|| {
                match_copy(proof, regions, graph, c, template, start, false)
                    .or_else(|| match_copy(proof, regions, graph, c, template, start, true))
                    .map(Rc::new)
            });
            if let Some(copy) = matched {
                cut = Some((index, Rc::clone(copy)));
                break;
            }
            if index < ir.blocks[node].statements.len()
                && crate::structure::statement_may_throw(ir, &ir.blocks[node].statements[index])
                && protected.is_none()
            {
                return None;
            }
        }
        if let Some((index, copy)) = cut {
            // A partial scope borrows the witness. It cannot erase shared
            // operations which still serve the remaining protected paths.
            if shared && !copy.nodes.is_disjoint(&template.nodes) {
                return None;
            }
            // Effects in a retained prefix still dispatch through the original
            // typed catches before reaching this finally.
            if ir.blocks[node].statements[..index]
                .iter()
                .any(|s| crate::structure::statement_may_throw(ir, s))
            {
                pending.extend(protected?.iter().map(|handler| handler.target));
            }
            copies.insert(node, (index, copy));
            continue;
        }
        if template.nodes.contains(&node) {
            return None;
        }
        if block.may_throw {
            pending.extend(protected?.iter().map(|handler| handler.target));
        }
        match block.end {
            FlowEnd::Return | FlowEnd::Unreachable => return None,
            FlowEnd::Throw if protected.is_none() => return None,
            _ => pending.extend(block.end.successors()),
        }
    }
    let independent_copies = copies
        .values()
        .filter(|(_, copy)| copy.evidence == CopyEvidence::Physical)
        .count();
    let owns_noop_exit = !shared
        && copies
            .values()
            .any(|(_, copy)| copy.evidence == CopyEvidence::SpecializedNoop);
    // A complete scope can own failure-only cleanup whose normal exits are
    // no-ops. Partial extraction needs physical copies: a no-op return cannot
    // establish borrowed ownership, and a shared join is ordinary continuation.
    // Interior partial scopes must consolidate multiple separate copies.
    if independent_copies == 0 && !owns_noop_exit
        || !shared && c.covered.iter().any(|node| !body_nodes.contains(node))
        || !boundary && independent_copies < 2
    {
        return None;
    }
    // Copies can share a continuation through empty corridors with other
    // incoming paths. Bypass those corridors without owning them, but retain
    // the copy boundary when the continuation reenters this protected body.
    let completion = |copy: &Copy| {
        normalized(
            ir,
            regions,
            graph,
            copy.end,
            &BTreeSet::new(),
            &mut BTreeSet::new(),
        )
        .filter(|end| !body_nodes.contains(&end.block))
        .unwrap_or(copy.end)
    };
    let mut endpoints = BTreeMap::new();
    let mut owned = body_nodes.clone();
    if !shared {
        owned.extend(&template.nodes);
    }
    for (_, copy) in copies.values() {
        let end = completion(copy);
        if endpoints
            .insert(end.block, end)
            .is_some_and(|old| old != end)
        {
            return None;
        }
        owned.extend(&copy.nodes);
    }
    // A source continuation retains the exact point after cleanup. Distinct
    // incoming edges may not enter its erased prefix or the middle of a copy.
    for (&node, end) in &endpoints {
        if node == root || template.nodes.contains(&node) && !template.completion.returns() {
            return None;
        }
        if body_nodes.contains(&node)
            && copies
                .get(&node)
                .is_none_or(|(index, _)| *index > end.index)
        {
            return None;
        }
    }
    for (&node, (_, copy)) in &copies {
        if copy.nodes.iter().any(|other| {
            body_nodes.contains(other) && *other != node && !endpoints.contains_key(other)
        }) {
            return None;
        }
    }
    let retained_entry = |node: BlockId| endpoints.get(&node).is_some_and(|point| point.index == 0);
    let mut entries = BTreeSet::new();
    if owned.contains(&graph.entry) && !retained_entry(graph.entry) {
        entries.insert(graph.entry);
    }
    for &node in &owned {
        if !retained_entry(node) && incoming[node].iter().any(|source| !owned.contains(source)) {
            entries.insert(node);
        }
    }
    // The throwing dominator seeds discovery, but a loop can return through a
    // nonthrowing prefix before that seed. Select the single actual entry of
    // the complete owned region; never admit an entry into cleanup or move an
    // exceptional catch binding into an ordinary source control edge.
    if entries.len() != 1 {
        return None;
    }
    let entry = *entries.first()?;
    if !body_nodes.contains(&entry)
        || (entry != root
            && (copies.contains_key(&entry) || ir.blocks[entry].caught_exception.is_some()))
    {
        return None;
    }
    // A retained continuation may loop back to the region's surviving entry,
    // but cannot enter an interior node consumed by this finally scope.
    for end in endpoints.values() {
        if graph.blocks[end.block]
            .end
            .successors()
            .iter()
            .any(|node| *node != entry && owned.contains(node) && !endpoints.contains_key(node))
        {
            return None;
        }
    }
    // The certificate uses effective dispatch. Source reconstruction retains
    // shadowed entries for catches owned by this body, but every external
    // handler belongs to the installed region's outer dispatch. An explicit
    // outer suffix need not occur here: a catch-all may reach it only through
    // its rethrow, or shadowed table entries may extend beyond that suffix.
    let mut sub = regions.flow.clone();
    let mut allowed: crate::index::IdVec<BlockId, _> = vec![false; sub.blocks.len()].into();
    for &node in &body_nodes {
        allowed[node] = true;
        sub.blocks[node]
            .handlers
            .retain(|handler| body_nodes.contains(&handler.target));
        if !sub.blocks[node].may_throw {
            sub.blocks[node].handlers.clear();
        }
    }
    let mut exits = Vec::new();
    let mut returns = Vec::new();
    let mut slices = BTreeMap::new();
    for (&node, (index, copy)) in &copies {
        let boundary = sub.blocks.next_id();
        sub.blocks.push(FlowBlock {
            pc: u32::MAX,
            statements_empty: false,
            duplication_cost: None,
            is_loop_update: false,
            end: FlowEnd::Unreachable,
            handlers: vec![],
            may_throw: false,
        });
        allowed.push(false);
        sub.blocks[node].end = FlowEnd::Goto(boundary);
        sub.blocks[node].may_throw = ir.blocks[node].statements[..*index]
            .iter()
            .any(|s| crate::structure::statement_may_throw(ir, s));
        if !sub.blocks[node].may_throw {
            sub.blocks[node].handlers.clear();
        }
        slices.insert(node, slice(ir, node, 0, *index));
        let target = if let Some(returning) = copy.pending_return {
            // Keep the semantic return separate from the original block's
            // terminal placeholder while its scheduled prefix is expanded.
            returns.push((boundary, vec![Stmt::Return(returning)]));
            boundary
        } else {
            completion(copy).block
        };
        exits.push((boundary, vec![Stmt::Jump(target)]));
    }
    // Ownership and copy identity above use the original ordered dispatch.
    // Java eligibility is applied only to this now-proved source view, before
    // shared handler continuations acquire their lexical control bindings.
    let mut effects = regions.replacements.clone();
    effects.extend(slices.iter().map(|(&node, body)| (node, body.clone())));
    let eligibility = crate::structure::eligibility::Eligibility::new(&sub, &effects, filter);
    let body = crate::structure::exception::subgraph_with_exits(
        &sub,
        entry,
        &exits,
        Some(&allowed),
        &[],
        0,
        false,
        &eligibility,
    )?;
    let body = crate::structure::installation::expand(body, &slices);
    let body = if returns.is_empty() {
        body
    } else {
        crate::structure::bindings::bind(body, &returns)
    };
    let mut plans = Vec::new();
    for &end in endpoints.values() {
        if template.completion.returns() {
            owned.remove(&end.block);
            continue;
        }
        let continuation = if end.index != 0 {
            Some(suffix(ir, regions, graph, end)?)
        } else {
            None
        };
        let (may_throw, handlers) = continuation.as_ref().map_or(
            (
                graph.blocks[end.block].may_throw,
                graph.blocks[end.block].handlers.as_slice(),
            ),
            |continuation| (continuation.may_throw, continuation.outer.as_slice()),
        );
        if may_throw
            && handlers.iter().any(|handler| {
                owned.contains(&handler.target) && !endpoints.contains_key(&handler.target)
            })
        {
            return None;
        }
        // An untouched continuation remains an ordinary graph node. Keeping
        // it opaque would unnecessarily hide an enclosing cleanup copy from
        // the next ownership proof.
        if let Some(continuation) = continuation {
            plans.push(continuation);
        }
        owned.remove(&end.block);
    }
    let targets = endpoints.keys().copied().map(|node| (node, node)).collect();
    let mut cleanups = super::super::cleanup::CleanupFacts::default();
    if !shared {
        cleanups.record(ir, c.handler, template.ends.iter().map(|point| point.block));
    }
    let cleanup = if filter.is_some() {
        cleanup_body(
            ir,
            regions,
            graph,
            c,
            &template.nodes,
            &template.ends,
            filter,
        )?
    } else {
        template.body.clone()
    };
    let body = template.completion.protect(body, c.handler);
    // All plans carry composed source bodies. Expand graph placeholders once
    // against the proof's region graph; retained suffixes already contain
    // their semantic atoms and must not be expanded again during installation.
    let body = crate::structure::installation::expand(
        vec![Stmt::TryFinally {
            body,
            cleanup,
            handler: c.handler,
        }],
        &regions.replacements,
    );
    plans.push(VerifiedRegion {
        entry,
        nodes: owned.into_iter().collect(),
        outer: c.outer.clone(),
        end: if template.completion.returns() {
            FlowEnd::Unreachable
        } else {
            FlowEnd::Multi(targets)
        },
        body,
        may_throw: true,
        cleanups,
    });
    Some(plans)
}
