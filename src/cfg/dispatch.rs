//! Ordered JVM exception dispatch. These rules describe executable edges, not
//! lexical protection: a shadowed outer table entry can still prove the scope
//! surrounding an inner catch and must remain available during reconstruction.
use super::HandlerEdge;
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
