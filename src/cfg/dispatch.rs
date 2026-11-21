//! Ordered JVM exception dispatch. These rules describe executable edges, not
//! lexical protection: a shadowed outer table entry can still prove the scope
//! surrounding an inner catch and must remain available during reconstruction.
use super::{BlockId, HandlerEdge};
use std::collections::BTreeSet;

pub fn is_catch_all(catch_type: Option<&str>) -> bool {
    catch_type.is_none_or(|ty| ty == "java/lang/Throwable")
}

/// Table indices distinguish input entries, but do not change dispatch.
pub fn same_handler(left: &HandlerEdge, right: &HandlerEdge) -> bool {
    left.target == right.target && left.catch_type == right.catch_type
}

pub fn same_dispatch(left: &[HandlerEdge], right: &[HandlerEdge]) -> bool {
    left.len() == right.len() && left.iter().zip(right).all(|(a, b)| same_handler(a, b))
}

/// Strip an enclosing dispatch shared by distinct exception-table ranges.
pub(crate) fn strip_outer<'a>(
    handlers: &'a [HandlerEdge],
    outer: &[HandlerEdge],
) -> Option<&'a [HandlerEdge]> {
    let split = handlers.len().checked_sub(outer.len())?;
    same_dispatch(&handlers[split..], outer).then_some(&handlers[..split])
}

/// Choose the deepest table shared by a strict majority of distinct protected
/// arms at that depth. Other arms can leave the proposed try. A tie supplies
/// no ownership evidence; repeated keys targeting one arm count once.
pub(crate) fn shared_arm<'a>(
    outer: &[HandlerEdge],
    arms: impl Iterator<Item = (BlockId, &'a [HandlerEdge])>,
) -> Option<BlockId> {
    let mut arms: Vec<_> = arms
        .filter(|(_, table)| !table.is_empty() && strip_outer(table, outer).is_some())
        .collect();
    arms.sort_unstable_by_key(|&(id, table)| (std::cmp::Reverse(table.len()), id));
    arms.dedup_by_key(|(id, _)| *id);
    if arms.len() < 2 {
        return None;
    }
    for layer in arms.chunk_by(|a, b| a.1.len() == b.1.len()) {
        // Cancel competing votes, then verify the survivor.
        let mut leader = layer[0];
        let mut votes = 0;
        for &arm in layer {
            if votes == 0 {
                leader = arm;
            }
            if same_dispatch(arm.1, leader.1) {
                votes += 1;
            } else {
                votes -= 1;
            }
        }
        let support = layer
            .iter()
            .filter(|(_, table)| same_dispatch(table, leader.1))
            .count();
        if support >= 2 && support > layer.len() / 2 {
            return Some(leader.0);
        }
    }
    // Distinct inner catches can all share a single exact outer anchor.
    let &(anchor, enclosing) = arms.last()?;
    arms.iter()
        .all(|(_, table)| strip_outer(table, enclosing).is_some())
        .then_some(anchor)
}

/// First matching handler wins. Without a hierarchy, only repeated types and
/// entries after a catch-all are provably unreachable.
pub fn effective(handlers: &[HandlerEdge]) -> impl Iterator<Item = &HandlerEdge> {
    let mut seen = BTreeSet::new();
    let mut complete = false;
    handlers.iter().filter(move |handler| {
        if complete {
            return false;
        }
        complete = is_catch_all(handler.catch_type.as_deref());
        seen.insert(handler.catch_type.as_deref())
    })
}

/// Executable dispatch with additional subtype evidence. The predicate answers
/// whether the later catch type is a subtype of an earlier one. Original table
/// entries remain untouched; source catch eligibility is a separate projection
/// over the effects of the final protected body.
pub fn effective_with<'a>(
    handlers: &'a [HandlerEdge],
    mut is_subtype: impl FnMut(&str, &str) -> bool + 'a,
) -> impl Iterator<Item = &'a HandlerEdge> {
    let mut seen = Vec::<&str>::new();
    effective(handlers).filter(move |handler| {
        if let Some(ty) = handler.catch_type.as_deref() {
            if seen.iter().any(|&earlier| is_subtype(ty, earlier)) {
                return false;
            }
            seen.push(ty);
        }
        true
    })
}

#[cfg(test)]
#[path = "../../tests/unit/dispatch.rs"]
mod tests;
