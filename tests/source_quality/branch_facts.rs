//! Normal completion and repeated reads refine the existing lexical facts.
use super::support::{
    Fixture, assert_absent, assert_contains, assert_no_word, compact, method, word_count,
};
use std::sync::OnceLock;

fn body(name: &str) -> &'static str {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    method(
        FIXTURE
            .get_or_init(|| {
                Fixture::build(
                    "tests/fixtures/source_quality/branch_facts",
                    "QualityBranchFacts",
                )
            })
            .source("QualityBranchFacts"),
        name,
    )
}

#[test]
fn completed_dereferences_remove_later_null_guards() {
    for name in ["completedReceiver", "completedArray"] {
        assert_no_word(body(name), "null");
        assert_absent(body(name), "if (");
        assert_contains(body(name), "length");
    }
}

#[test]
fn conditional_reads_writes_and_handlers_do_not_prove_nonnull() {
    for name in [
        "conditionalReceiver",
        "overwrittenReceiver",
        "caughtReceiver",
    ] {
        assert!(word_count(body(name), "null") > 0, "{}", body(name));
    }
}

#[test]
fn unchanged_fields_share_guard_facts() {
    for name in ["clamp", "instanceClamp"] {
        let body = body(name);
        assert_eq!(compact(body).matches("if(").count(), 1, "{body}");
        assert_absent(body, "<= 0");
        assert_contains(body, "--");
    }
}

#[test]
fn heap_mutation_and_synchronization_kill_field_facts() {
    for name in [
        "callChangesField",
        "writeChangesField",
        "volatileReads",
        "monitorBoundary",
        "initializerChangesField",
    ] {
        assert_contains(body(name), "<= 0");
    }
}

#[test]
fn known_nonzero_masks_discard_only_completed_plain_reads() {
    assert_absent(body("nonzeroMask"), "|");
    assert_absent(body("nonzeroMask"), "return 4");
    assert_contains(body("effectfulMask"), "mutate()");
    assert_contains(body("volatileMask"), "changing | 1");
}
