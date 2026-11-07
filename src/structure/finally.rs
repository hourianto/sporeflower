//! Prove cleanup ownership on the scheduled graph before handler lowering;
//! retain lexical recovery for already bounded regions. Every outward normal
//! transfer must already execute equivalent cleanup. Internal loop backedges
//! are not transfers out of the protected region.
mod completion;
mod equivalent;
mod exits;
mod region;
mod specialization;
#[cfg(test)]
#[path = "../../tests/unit/cleanup_analysis.rs"]
mod tests;
pub(super) use region::plan_candidate;

use super::Stmt;
use crate::index::BlockId;
use crate::ir::{MethodIr, Terminator};
use std::{cell::OnceCell, collections::BTreeSet};

/// Copy contents belong to this exact immutable scheduled graph. Region
/// composition and lexical recovery change only views of it, so all cleanup
/// candidates share one lazy analysis until this method's structuring ends.
pub(super) struct Analysis<'a> {
    ir: &'a MethodIr,
    available: OnceCell<crate::ir::AvailableCopies>,
}

impl<'a> Analysis<'a> {
    pub fn new(ir: &'a MethodIr) -> Self {
        Self {
            ir,
            available: OnceCell::new(),
        }
    }

    pub fn ir(&self) -> &'a MethodIr {
        self.ir
    }

    fn available(&self) -> &crate::ir::AvailableCopies {
        self.available
            .get_or_init(|| crate::ir::AvailableCopies::analyze(self.ir))
    }

    fn same(&self, left: &[Stmt], right: &[Stmt]) -> bool {
        equivalent::same(self, left, right)
    }
}

pub(super) fn recover(analysis: &Analysis<'_>, body: Vec<Stmt>) -> Vec<Stmt> {
    if !super::tree::walk(&body).any(|statement| {
        matches!(statement, Stmt::Try { catches, .. }
            if catches.last().is_some_and(|catch|
                crate::cfg::dispatch::is_catch_all(catch.catch_type.as_deref())))
    }) {
        return body;
    }
    recover_region(analysis, body, &exits::Bindings::new(), None)
}

fn recover_region(
    proof: &Analysis<'_>,
    body: Vec<Stmt>,
    bindings: &exits::Bindings,
    continuation: Option<&Stmt>,
) -> Vec<Stmt> {
    let ir = proof.ir;
    // Compute transparent continuations once before consuming the tree. Only
    // terminal transfer identities are copied; nested source bodies move once.
    let mut next = continuation.cloned();
    let mut following = Vec::with_capacity(body.len());
    for statement in body.iter().rev() {
        following.push(next.clone());
        next =
            exits::transparent_follow(ir, std::slice::from_ref(statement), next.as_ref(), bindings);
    }
    let mut result = Vec::with_capacity(body.len());
    for (statement, following) in body.into_iter().zip(following.into_iter().rev()) {
        let nested = statement.binder().map(|binder| {
            let mut nested = bindings.clone();
            nested.remove(&binder.region());
            if let Some(exit) = &following {
                nested.insert(binder.region(), exit.clone());
            }
            nested
        });
        result.push(children(
            proof,
            statement,
            nested.as_ref().unwrap_or(bindings),
            following.as_ref(),
        ));
        // Recovered exits can make a child terminal even when its incoming
        // tree used a shared continuation. Keep lexical sequences valid as
        // that information becomes available to enclosing regions.
        if !super::can_complete(&result[result.len() - 1..]) {
            break;
        }
    }
    let mut body = result;
    let mut index = 0;
    while index < body.len() {
        if let Some((replacement, consumed)) =
            candidate(proof, &body[index..], bindings, continuation)
        {
            body.splice(index..index + consumed, replacement);
        }
        index += 1;
    }
    if let Some(terminal) = body
        .iter()
        .position(|statement| !super::can_complete(std::slice::from_ref(statement)))
    {
        body.truncate(terminal + 1);
    }
    body
}

fn children(
    proof: &Analysis<'_>,
    statement: Stmt,
    bindings: &exits::Bindings,
    continuation: Option<&Stmt>,
) -> Stmt {
    let continuation = match &statement {
        Stmt::If { .. } | Stmt::Label { .. } | Stmt::Try { .. } => continuation,
        _ => None,
    };
    statement.map_bodies(|body| recover_region(proof, body, bindings, continuation))
}

fn candidate(
    proof: &Analysis<'_>,
    sequence: &[Stmt],
    bindings: &exits::Bindings,
    continuation: Option<&Stmt>,
) -> Option<(Vec<Stmt>, usize)> {
    let ir = proof.ir;
    let Stmt::Try { body, catches } = sequence.first()? else {
        return None;
    };
    let catch = catches.last()?;
    if catch
        .catch_type
        .as_deref()
        .is_some_and(|ty| ty != "java/lang/Throwable")
    {
        return None;
    }
    let (last, cleanup) = catch.body.split_last()?;
    let Stmt::Throw(block) = last else {
        return None;
    };
    let Terminator::Throw { value, .. } = ir.blocks[*block].terminator else {
        return None;
    };
    // Empty scheduled fragments are not a cleanup. This proof runs before
    // source normalization removes them; a plain catch-and-rethrow must keep
    // its exception-flow meaning for checked-exception inference.
    if ir.blocks[catch.handler].caught_exception != Some(value)
        || cleanup.iter().all(|statement| {
            statement
                .scheduled(ir)
                .is_some_and(|scheduled| scheduled.statements.is_empty())
        })
    {
        return None;
    }
    // The caught value cannot be read by a Java finally body, and a cleanup
    // return must never replace a pending return or exception accidentally.
    if !cleanup_control(cleanup)
        || equivalent::reads_value(ir, cleanup, value)
        || !super::can_complete(cleanup)
    {
        return None;
    }
    let mut copies = 0;
    let body = strip_exits(
        proof,
        body,
        cleanup,
        &BTreeSet::new(),
        &BTreeSet::new(),
        bindings,
        catch.handler,
        &mut copies,
    )?;
    let mut retained = Vec::new();
    for prior in &catches[..catches.len() - 1] {
        let rewritten = strip_exits(
            proof,
            &prior.body,
            cleanup,
            &BTreeSet::new(),
            &BTreeSet::new(),
            bindings,
            catch.handler,
            &mut copies,
        )?;
        // Earlier catches must already dispatch their escaping effects to the
        // same cleanup before a finally may enclose them.
        if !handler_coverage(ir, &rewritten, catch.handler, true) {
            return None;
        }
        retained.push(super::Catch {
            body: rewritten,
            ..prior.clone()
        });
    }
    let falls_through = super::can_complete(&body)
        || retained
            .iter()
            .any(|catch| super::can_complete(&catch.body));
    let (body, consumed, continuation) = if falls_through {
        normal_copy(
            proof,
            body,
            &retained,
            cleanup,
            &sequence[1..],
            bindings,
            catch.handler,
            continuation,
        )?
    } else {
        (body, 1, None)
    };
    if falls_through {
        copies += 1;
    }
    if copies == 0 {
        return None;
    }
    let protected = if retained.is_empty() {
        body
    } else {
        vec![Stmt::Try {
            body,
            catches: retained,
        }]
    };
    let mut replacement = vec![Stmt::TryFinally {
        body: protected,
        cleanup: cleanup.to_vec(),
        handler: catch.handler,
    }];
    replacement.extend(continuation);
    Some((replacement, consumed))
}

#[expect(
    clippy::too_many_arguments,
    reason = "Cleanup matching needs the protected body, following sequence, and lexical exit context"
)]
fn normal_copy(
    proof: &Analysis<'_>,
    body: Vec<Stmt>,
    catches: &[super::Catch],
    cleanup: &[Stmt],
    following: &[Stmt],
    bindings: &exits::Bindings,
    handler: BlockId,
    continuation: Option<&Stmt>,
) -> Option<(Vec<Stmt>, usize, Option<Stmt>)> {
    let ir = proof.ir;
    let mut alternatives = vec![(body.clone(), Vec::new())];
    if catches
        .iter()
        .all(|catch| !super::can_complete(&catch.body))
        && let Some(scheduled) = body.last().and_then(|statement| statement.scheduled(ir))
    {
        for cut in scheduled.start..scheduled.range().end {
            let suffix = Stmt::Statements {
                block: scheduled.block,
                start: cut,
                end: scheduled.range().end,
            };
            if suffix
                .scheduled(ir)
                .unwrap()
                .iter()
                .any(|(_, statement)| super::statement_may_throw(ir, statement))
            {
                continue;
            }
            let mut prefix = body[..body.len() - 1].to_vec();
            if cut != scheduled.start {
                prefix.push(Stmt::Statements {
                    block: scheduled.block,
                    start: scheduled.start,
                    end: cut,
                });
            }
            alternatives.push((prefix, vec![suffix]));
        }
    }
    for (body, prefix) in alternatives {
        let extra = prefix.len();
        let actual: Vec<_> = prefix
            .into_iter()
            .chain(following.iter().cloned())
            .collect();
        let direct = cleanup.len();
        if actual.len() >= direct
            && direct >= extra
            && proof.same(cleanup, &actual[..direct])
            && outside_handler(ir, &actual[..direct], handler)
        {
            return Some((body, 1 + direct - extra, None));
        }
        for end in extra.max(1)..=actual.len() {
            let implicit = (end == actual.len()).then_some(continuation).flatten();
            let Some(exit) = exits::completion(ir, &actual[..end], implicit, bindings) else {
                continue;
            };
            if let Some(normalized) = exits::normalize(ir, &actual[..end], &exit, bindings)
                && proof.same(cleanup, &normalized)
                && outside_handler(ir, &normalized, handler)
            {
                return Some((
                    body,
                    1 + end - extra,
                    if implicit.is_some_and(|following| equivalent::same_exit(ir, &exit, following))
                    {
                        None
                    } else {
                        Some(exit)
                    },
                ));
            }
        }
    }
    None
}

fn outside_handler(ir: &MethodIr, body: &[Stmt], handler: BlockId) -> bool {
    handler_coverage(ir, body, handler, false)
}

fn handler_coverage(ir: &MethodIr, body: &[Stmt], handler: BlockId, required: bool) -> bool {
    let mut valid = true;
    visit(body, &mut |statement| {
        let mut throwing = Vec::new();
        if let Some(scheduled) = statement.scheduled(ir)
            && scheduled
                .iter()
                .any(|(_, statement)| super::statement_may_throw(ir, statement))
        {
            throwing.push(scheduled.block);
        }
        match statement {
            Stmt::Throw(block) => throwing.push(*block),
            Stmt::If { condition, .. } => {
                for block in condition.heads() {
                    if super::values_may_throw(
                        ir,
                        ir.blocks[block]
                            .terminator
                            .operands()
                            .into_iter()
                            .map(|value| (value, false))
                            .collect(),
                    ) {
                        throwing.push(block);
                    }
                }
            }
            Stmt::Switch { head, .. }
                if super::values_may_throw(
                    ir,
                    ir.blocks[*head]
                        .terminator
                        .operands()
                        .into_iter()
                        .map(|value| (value, false))
                        .collect(),
                ) =>
            {
                throwing.push(*head);
            }
            _ => (),
        }
        if throwing.into_iter().any(|block| {
            crate::cfg::dispatch::effective(&ir.blocks[block].handlers)
                .any(|edge| edge.target == handler)
                != required
        }) {
            valid = false;
        }
    });
    valid
}

#[expect(
    clippy::too_many_arguments,
    reason = "Recursive cleanup matching carries distinct region, loop, binding, and copy-count state"
)]
fn strip_exits(
    proof: &Analysis<'_>,
    body: &[Stmt],
    cleanup: &[Stmt],
    regions: &BTreeSet<usize>,
    loops: &BTreeSet<BlockId>,
    bindings: &exits::Bindings,
    handler: BlockId,
    copies: &mut usize,
) -> Option<Vec<Stmt>> {
    let ir = proof.ir;
    if let Some(expected) = exits::completion(ir, body, None, bindings) {
        let outward = match &expected {
            Stmt::Return(_) => true,
            Stmt::Break { target } => !regions.contains(target),
            Stmt::Continue { header } => !loops.contains(header),
            _ => false,
        };
        if outward {
            for start in 0..body.len() {
                let cuts = body[start].scheduled(ir).map_or_else(
                    || vec![None],
                    |scheduled| (scheduled.start..scheduled.range().end).map(Some).collect(),
                );
                for cut in cuts {
                    let mut actual = body[start..].to_vec();
                    let mut prefix = body[..start].to_vec();
                    if let Some(cut) = cut {
                        let scheduled = body[start].scheduled(ir).unwrap();
                        actual[0] = Stmt::Statements {
                            block: scheduled.block,
                            start: cut,
                            end: scheduled.range().end,
                        };
                        if cut > scheduled.start {
                            prefix.push(Stmt::Statements {
                                block: scheduled.block,
                                start: scheduled.start,
                                end: cut,
                            });
                        }
                    }
                    let Some(normalized) = exits::normalize(ir, &actual, &expected, bindings)
                    else {
                        continue;
                    };
                    if !proof.same(cleanup, &normalized)
                        || !outside_handler(ir, &normalized, handler)
                    {
                        continue;
                    }
                    if let Stmt::Return(block) = &expected
                        && let Terminator::Return(Some(value)) = ir.blocks[*block].terminator
                        && !equivalent::stable_return(ir, value, &normalized)
                    {
                        continue;
                    }
                    let mut result = strip_exits(
                        proof, &prefix, cleanup, regions, loops, bindings, handler, copies,
                    )?;
                    result.push(expected);
                    *copies += 1;
                    return Some(result);
                }
            }
        }
    }
    let mut result = Vec::new();
    for statement in body {
        let outward = match statement {
            Stmt::Return(_) => true,
            Stmt::Break { target } => !regions.contains(target),
            Stmt::Continue { header } => !loops.contains(header),
            Stmt::Jump(_)
            | Stmt::DispatchContinue { .. }
            | Stmt::BoundRegion { .. }
            | Stmt::Dispatch { .. } => return None,
            _ => false,
        };
        if outward {
            let start = result.len().checked_sub(cleanup.len())?;
            let actual = &result[start..];
            if !proof.same(cleanup, actual) || !outside_handler(ir, actual, handler) {
                return None;
            }
            if let Stmt::Return(block) = statement
                && let Terminator::Return(Some(value)) = ir.blocks[*block].terminator
                && !equivalent::stable_return(ir, value, actual)
            {
                return None;
            }
            result.truncate(start);
            *copies += 1;
            result.push(statement.clone());
            continue;
        }
        result.push(match statement {
            Stmt::If {
                condition,
                then_body,
                else_body,
            } => Stmt::If {
                condition: condition.clone(),
                then_body: strip_exits(
                    proof, then_body, cleanup, regions, loops, bindings, handler, copies,
                )?,
                else_body: strip_exits(
                    proof, else_body, cleanup, regions, loops, bindings, handler, copies,
                )?,
            },
            Stmt::Loop {
                region,
                header,
                body,
                continuation,
            } => {
                let mut regions = regions.clone();
                regions.insert(*region);
                let mut loops = loops.clone();
                loops.insert(*header);
                Stmt::Loop {
                    region: *region,
                    header: *header,
                    body: strip_exits(
                        proof, body, cleanup, &regions, &loops, bindings, handler, copies,
                    )?,
                    continuation: *continuation,
                }
            }
            Stmt::Label { region, body } => {
                let mut regions = regions.clone();
                regions.insert(*region);
                Stmt::Label {
                    region: *region,
                    body: strip_exits(
                        proof, body, cleanup, &regions, loops, bindings, handler, copies,
                    )?,
                }
            }
            Stmt::Switch {
                region,
                head,
                cases,
            } => {
                let mut regions = regions.clone();
                regions.insert(*region);
                Stmt::Switch {
                    region: *region,
                    head: *head,
                    cases: cases
                        .iter()
                        .map(|case| {
                            Some(super::SwitchCase {
                                body: strip_exits(
                                    proof, &case.body, cleanup, &regions, loops, bindings, handler,
                                    copies,
                                )?,
                                ..case.clone()
                            })
                        })
                        .collect::<Option<_>>()?,
                }
            }
            Stmt::Try { body, catches } => Stmt::Try {
                body: strip_exits(
                    proof, body, cleanup, regions, loops, bindings, handler, copies,
                )?,
                catches: catches
                    .iter()
                    .map(|catch| {
                        Some(super::Catch {
                            body: strip_exits(
                                proof,
                                &catch.body,
                                cleanup,
                                regions,
                                loops,
                                bindings,
                                handler,
                                copies,
                            )?,
                            ..catch.clone()
                        })
                    })
                    .collect::<Option<_>>()?,
            },
            // Moving an outer cleanup across another implicit cleanup needs a
            // composed exit proof; preserve those nested regions for now.
            Stmt::TryFinally { .. } | Stmt::Synchronized { .. } => return None,
            other => other.clone(),
        });
    }
    Some(result)
}

fn cleanup_control(body: &[Stmt]) -> bool {
    use super::tree::Binder;
    let mut valid = true;
    super::tree::walk_scoped(body, &mut |statement, scope| {
        valid &= match statement {
            Stmt::Basic(_)
            | Stmt::Statements { .. }
            | Stmt::Throw(_)
            | Stmt::If { .. }
            | Stmt::Loop { .. }
            | Stmt::Label { .. }
            | Stmt::Switch { .. }
            | Stmt::Try { .. }
            | Stmt::TryFinally { .. }
            | Stmt::Synchronized { .. } => true,
            Stmt::Break { target } => scope.iter().rev().any(|binder| binder.region() == *target),
            Stmt::Continue { header } => scope.iter().rev().any(
                |binder| matches!(binder, Binder::Loop { header: owner, .. } if owner == header),
            ),
            _ => false,
        };
    });
    valid
}

fn visit(body: &[Stmt], function: &mut impl FnMut(&Stmt)) {
    for statement in super::tree::walk(body) {
        function(statement);
        // An owned loop update is scheduled outside its lexical body.
        if let Stmt::Loop {
            continuation: Some(block),
            ..
        } = statement
        {
            function(&Stmt::Basic(*block));
        }
    }
}
