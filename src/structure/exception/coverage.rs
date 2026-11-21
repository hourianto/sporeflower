//! Can these additional lexical handlers change JVM dispatch at this block?
use crate::cfg::HandlerEdge;
use crate::cfg::dispatch::{same_dispatch, same_handler};
use crate::index::BlockId;
use crate::ir::failures;
use crate::structure::analysis::MethodAnalysis;

pub(in crate::structure) fn admits(
    analysis: &MethodAnalysis<'_>,
    block: BlockId,
    short: &[HandlerEdge],
    long: &[HandlerEdge],
) -> bool {
    let Some(facts) = analysis.method else {
        return false;
    };
    if short.len() >= long.len() || !facts.can_use_platform_contract("java/lang/Exception") {
        return false;
    }
    let mut retained = short.iter().peekable();
    let mut unchanged_errors = true;
    let mut unchanged_runtime = true;
    for edge in long {
        if retained.next_if(|h| same_handler(h, edge)).is_some() {
            continue;
        }
        let disjoint = |family| {
            edge.catch_type.as_deref().is_some_and(|ty| {
                facts.complete_hierarchy(ty)
                    && facts.complete_hierarchy(family)
                    && !facts.is_subtype(ty, family)
                    && !facts.is_subtype(family, ty)
            })
        };
        unchanged_errors &= disjoint("java/lang/Error");
        unchanged_runtime &= disjoint("java/lang/RuntimeException");
    }
    if retained.next().is_some() {
        return false;
    }

    let prefix = same_dispatch(short, &long[..short.len()]);
    let same_runtime = |ty| {
        // A bundled profile can omit a concrete VM exception. Its exact class
        // is unnecessary when no added handler can catch the runtime family.
        if unchanged_runtime
            && facts.can_use_platform_contract(ty)
            && facts.can_use_platform_contract("java/lang/RuntimeException")
        {
            return true;
        }
        let catches = |h: &&HandlerEdge| {
            h.catch_type
                .as_deref()
                .is_none_or(|t| facts.is_subtype(ty, t))
        };
        let original = short.iter().find(catches);
        if prefix && original.is_some() {
            return true;
        }
        facts.complete_hierarchy(ty)
            && match (original, long.iter().find(catches)) {
                (Some(a), Some(b)) => same_handler(a, b),
                (None, None) => true,
                _ => false,
            }
    };
    let changed = |failure, point| {
        analysis.may_throw_at(failure, point, !unchanged_errors, |ty| !same_runtime(ty))
    };
    let ir = analysis.ir;
    let body = &ir.blocks[block];
    !body
        .statements
        .iter()
        .enumerate()
        .any(|(i, stmt)| failures::statement(ir, stmt, |f| changed(f, (block, i))))
        && !failures::terminator(ir, &body.terminator, |f| {
            changed(f, (block, body.statements.len()))
        })
}
