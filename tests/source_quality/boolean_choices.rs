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
        assert_absent(body, "true &");
        assert_absent(body, "!!");
    }
    assert_eq!(
        body("alternateToggle").matches('&').count(),
        0,
        "{}",
        body("alternateToggle")
    );
}

#[test]
fn known_boolean_inputs_remove_the_identity_mask() {
    let body = body("maskedChoice");
    assert_contains(body, "selected && raw");
    assert_absent(body, "?");
}

#[test]
fn negated_conjunctions_bind_as_logical_conditions() {
    let body = body("missingMask");
    assert_contains(body, "||");
    assert_absent(body, "?");
    assert_absent(body, "true");
}

#[test]
fn substituted_predicates_keep_direct_comparisons() {
    let body = body("substitutedMask");
    assert_contains(body, "if ((value & 16) == 0)");
    assert_absent(body, "boolean ");
    assert_absent(body, "!(");
}

#[test]
fn ordered_bounds_do_not_outvote_a_negative_guard() {
    let body = body("orderedGuard");
    assert_contains(body, "if (ready && value >= 0 && value < limit)");
    assert_absent(body, "||");
}

#[test]
fn compound_loop_conditions_expose_each_test() {
    let body = body("compoundLoop");
    assert_contains(body, "value < limit && (flags & 16) != 0");
    assert_absent(body, "!(");
    assert_absent(body, "while (true)");
}

#[test]
fn newly_selected_call_arms_keep_their_truth_test_boundary() {
    let body = body("selectedCalls");
    assert_eq!(body.matches('?').count(), 1, "{body}");
    assert_eq!(compact(body).matches("probe(").count(), 2, "{body}");
    assert_absent(body, "!!");
    assert_absent(body, "? true : false");
}

#[test]
fn predicate_consumers_remove_numeric_and_boolean_normalization() {
    for name in ["canonicalMask", "canonicalEquality"] {
        let body = body(name);
        assert_absent(body, "?");
        assert_absent(body, "true");
        assert_absent(body, "!!");
        assert_contains(body, "probe(value)");
    }
    let body = self::body("normalizedEquality");
    assert_absent(body, "1");
    assert_absent(body, "0");
    assert_contains(body, "return flag;");
    let body = self::body("fieldPredicate");
    assert_contains(body, "selected ||");
    assert_absent(body, "?");
    let body = self::body("virtualPredicates");
    assert_contains(body, "virtualProbe(first) &&");
    assert_absent(body, "?");
    assert_absent(body, "!!");
}

#[test]
fn eager_boolean_identities_keep_only_the_meaningful_operands() {
    let body = body("predicateMask");
    assert_contains(body, "(value & 2) == 0");
    assert_absent(body, "true &");
    let body = self::body("eagerSeed");
    assert_absent(body, "false |");
    assert_absent(body, "||");
    assert_eq!(compact(body).matches("probe(").count(), 2, "{body}");
}

#[test]
fn genuine_choices_use_positive_selectors() {
    assert_contains(body("selectedText"), "selected ? second : first");
    assert_contains(
        body("opponent"),
        "player == players[0] ? players[1] : players[0]",
    );
    assert_contains(body("nestedText"), "!selected ? \"default\" :");
}

#[test]
fn established_guards_remove_redundant_inner_choices() {
    let body = body("guardedEquals");
    assert_contains(
        body,
        "first == null ? second == null : first.equals(second)",
    );
    assert_eq!(body.matches('?').count(), 1, "{body}");
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

#[test]
fn complete_protected_return_operands_choose_short_circuit_expressions() {
    let alternative = body("protectedAlternative");
    assert_contains(alternative, "value != 0 ||");
    assert_contains(alternative, "probe(value + 1)");
    assert_absent(alternative, "if (");
    assert_absent(alternative, "boolean ");
    let conjunction = body("protectedConjunction");
    assert_eq!(conjunction.matches("&&").count(), 3, "{conjunction}");
    assert_absent(conjunction, "if (");
    assert_absent(conjunction, "boolean ");
}
