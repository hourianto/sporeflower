//! Trivial counted loops expose their useful work, with runtime counterexamples
//! for observed controls, effects, integer wraparound and transfer ownership.
use super::support::{Fixture, assert_absent, assert_contains, assert_no_word, compact, method};
use std::sync::OnceLock;

fn fixture(stripped: bool) -> &'static Fixture {
    static DEBUG: OnceLock<Fixture> = OnceLock::new();
    static STRIPPED: OnceLock<Fixture> = OnceLock::new();
    let storage = if stripped { &STRIPPED } else { &DEBUG };
    storage.get_or_init(|| {
        let directory = "tests/fixtures/source_quality/loops";
        if stripped {
            Fixture::build_stripped(directory, "QualityLoops")
        } else {
            Fixture::build(directory, "QualityLoops")
        }
    })
}

fn body(name: &str, stripped: bool) -> &'static str {
    method(fixture(stripped).source("QualityLoops"), name)
}

fn no_loop(body: &str) {
    for syntax in ["for (", "while (", "do {", "switch ("] {
        assert_absent(body, syntax);
    }
}

#[test]
fn private_single_trip_controls_leave_only_their_body() {
    for stripped in [false, true] {
        for name in [
            "ascendingOnce",
            "inclusiveOnce",
            "strideOnce",
            "subtractOnce",
            "wrappingOnce",
        ] {
            let body = body(name, stripped);
            no_loop(body);
            assert_no_word(body, "int");
            assert_eq!(body.matches("mark(").count(), 1, "{name}: {body}");
        }
    }
}

#[test]
fn aoe_style_draw_wrapper_does_not_obscure_the_real_iteration() {
    for stripped in [false, true] {
        let body = body("drawOnce", stripped);
        assert_eq!(compact(body).matches("for(").count(), 1, "{body}");
        assert_contains(body, ".length - 1;");
        assert_eq!(body.matches("mark(").count(), 2, "{body}");
    }
}

#[test]
fn zero_and_empty_single_trip_loops_disappear() {
    for name in ["zeroTrips", "emptyOnce"] {
        let body = body(name, false);
        no_loop(body);
        assert_eq!(body.matches("mark(").count(), 1, "{body}");
        assert_no_word(body, "int");
    }
    assert_absent(body("zeroTrips", false), "mark(17)");
}

#[test]
fn late_exhaustion_returns_do_not_restore_trivial_repetition() {
    for stripped in [false, true] {
        let once = body("selectedOnce", stripped);
        no_loop(once);
        assert_eq!(once.matches("mark(").count(), 1, "{once}");
        let zero = body("selectedZeroTrips", stripped);
        no_loop(zero);
        assert_absent(zero, "mark(");
    }
}

#[test]
fn counter_reads_writes_and_escaping_values_keep_the_loop() {
    for name in ["observedCounter", "modifiedCounter", "escapingCounter"] {
        assert_contains(body(name, false), "for (");
    }
}

#[test]
fn control_effects_are_never_discarded() {
    for name in [
        "effectfulInitializer",
        "effectfulCondition",
        "effectfulUpdate",
    ] {
        let body = body(name, false);
        assert!(body.contains("for (") || body.contains("while ("), "{body}");
        assert_eq!(body.matches("mark(").count(), 2, "{body}");
    }
}

#[test]
fn owned_transfers_keep_their_loop_target() {
    // A short tail no longer needs a continue. The pre-existing private-counter
    // proof can then remove this one-trip loop without losing its selected work.
    let continued = body("ownedContinue", false);
    no_loop(continued);
    assert_absent(continued, "continue;");
    assert_contains(
        continued,
        "QualityLoops.mark(26); if (!skip) { QualityLoops.mark(27); QualityLoops.mark(28); }",
    );
    assert_eq!(continued.matches("mark(").count(), 3, "{continued}");

    let broken = body("ownedBreak", false);
    assert_contains(broken, "for (");
    assert_contains(broken, "break;");
    assert_eq!(broken.matches("mark(").count(), 3, "{broken}");
    let labelled = body("labelledContinue", false);
    assert_eq!(compact(labelled).matches("for(").count(), 2, "{labelled}");
    let target = labelled
        .lines()
        .find_map(|line| line.trim().strip_prefix("continue "))
        .and_then(|target| target.strip_suffix(';'))
        .filter(|target| !target.is_empty())
        .unwrap_or_else(|| panic!("missing outer continue: {labelled}"));
    assert_contains(labelled, &format!("{target}: for ("));
    assert_eq!(labelled.matches("mark(").count(), 2, "{labelled}");
    assert_contains(labelled, "QualityLoops.mark(33);");
}

#[test]
fn nested_loop_transfers_do_not_pin_an_unobserved_outer_counter() {
    let body = body("nestedTransfers", false);
    assert_eq!(compact(body).matches("for(").count(), 1, "{body}");
    assert_absent(body, "continue;");
    assert_contains(body, "index < values.length && values[index] >= 0");
    assert_contains(
        body,
        "if (values[index] != 0) { QualityLoops.mark(values[index]); QualityLoops.mark(32); }",
    );
    assert_eq!(body.matches("mark(").count(), 2, "{body}");
}

#[test]
fn protected_body_and_cleanup_keep_their_original_ownership() {
    let body = body("protectedOnce", false);
    assert_contains(body, "try {");
    assert_contains(body, "catch (");
    assert_contains(body, "finally {");
    assert_eq!(body.matches("mark(").count(), 3, "{body}");
}

#[test]
fn multiple_trips_and_wrapped_repetition_remain_loops() {
    for name in ["twoTrips", "wrapDoesNotExit"] {
        assert_contains(body(name, false), "for (");
    }
}

#[test]
fn elided_loop_bindings_do_not_collide_with_following_scopes() {
    let body = body("localScopesOnce", false);
    no_loop(body);
    assert_eq!(body.matches("mark(").count(), 4, "{body}");
}
