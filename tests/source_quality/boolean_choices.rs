//! Seeded eager accumulators and complementary guards share boolean facts.
use super::support::{Fixture, assert_absent, assert_contains, compact, method};
use std::sync::OnceLock;

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        Fixture::build(
            "tests/fixtures/source_quality/boolean_choices",
            "QualityBooleanChoices",
        )
    })
}

fn body(name: &str) -> &'static str {
    method(fixture().source("QualityBooleanChoices"), name)
}

fn assert_eager_and(body: &str) {
    assert!(body.split("&&").any(|part| part.contains('&')), "{body}");
}

#[test]
fn complementary_local_guards_retain_a_conditional_choice() {
    for name in ["toggle", "alternateToggle"] {
        let body = body(name);
        assert_eq!(body.matches('?').count(), 1, "{body}");
        assert_absent(body, "||");
        assert_absent(body, "&&");
    }
    assert_eq!(
        body("alternateToggle").matches('&').count(),
        1,
        "{}",
        body("alternateToggle")
    );
}

#[test]
fn masks_prove_canonical_results_without_normalizing_raw_inputs() {
    let body = body("maskedChoice");
    assert_contains(body, "selected &&");
    assert_contains(body, "true & raw");
    assert_absent(body, "?");
}

#[test]
fn newly_selected_call_arms_keep_their_truth_test_boundary() {
    let body = body("selectedCalls");
    assert_eq!(body.matches('?').count(), 1, "{body}");
    assert_eq!(compact(body).matches("probe(").count(), 2, "{body}");
    assert_contains(body, "!!QualityBooleanChoices.probe(first)");
}

#[test]
fn eager_accumulators_and_observable_tails_keep_their_evaluations() {
    for name in ["eagerFailure", "eagerTail", "changingGuard", "volatileTail"] {
        let body = body(name);
        assert_absent(body, "?");
        assert_eager_and(body);
    }
    assert_contains(body("eagerSuccess"), "true |");
    let body = body("readiness");
    assert_eq!(compact(body).matches("probe(").count(), 3, "{body}");
    assert_eager_and(body);
}
