//! Identical phi-edge stores belong to the decision's normal continuation.
use super::support::{
    Fixture, assert_absent, assert_contains, assert_no_word, compact, method, word_count,
};
use std::sync::OnceLock;

fn fixture(stripped: bool) -> &'static Fixture {
    static NAMED: OnceLock<Fixture> = OnceLock::new();
    static STRIPPED: OnceLock<Fixture> = OnceLock::new();
    if stripped {
        STRIPPED.get_or_init(|| {
            Fixture::build_stripped(
                "tests/fixtures/source_quality/shared_tails",
                "QualitySharedTails",
            )
        })
    } else {
        NAMED.get_or_init(|| {
            Fixture::build(
                "tests/fixtures/source_quality/shared_tails",
                "QualitySharedTails",
            )
        })
    }
}

#[test]
fn nested_direction_decisions_share_one_length_store() {
    let body = method(fixture(false).source("QualitySharedTails"), "spiral");
    assert_eq!(
        compact(body).matches("remaining=length;").count(),
        1,
        "{body}"
    );
    assert_contains(body, "if (direction < 0) { direction += 8; }");
    assert_absent(body, "if (direction >= 0)");
}

#[test]
fn unchanged_coordinate_keeps_guarded_direction_updates() {
    let body = method(fixture(false).source("QualitySharedTails"), "wrap");
    assert_contains(body, "switch (direction)");
    assert_no_word(body, "else");
    assert_eq!(body.matches("if (").count(), 3, "{body}");
}

#[test]
fn protected_scan_keeps_only_the_effectful_branch() {
    let body = method(fixture(false).source("QualitySharedTails"), "protectedScan");
    assert_contains(body, "if (skip)");
    assert_contains(body, "catch (IllegalArgumentException");
    assert_no_word(body, "else");
}

#[test]
fn constructed_result_is_committed_once_after_conditional_setup() {
    let body = method(fixture(false).source("QualitySharedTails"), "decode");
    assert_eq!(
        compact(body).matches("result=created;").count(),
        1,
        "{body}"
    );
    assert_contains(body, "created.value = seed;");
}

#[test]
fn stripped_copies_recover_guards_without_debug_name_hints() {
    let source = fixture(true).source("QualitySharedTails");
    let spiral = method(source, "spiral");
    assert_eq!(word_count(spiral, "else"), 1, "{spiral}");
    assert_no_word(method(source, "wrap"), "else");
    assert_no_word(method(source, "protectedScan"), "else");
}
