//! Search exits own their effects, while an exhausted scan falls into its fallback.
use super::support::{Fixture, assert_absent, assert_no_word, compact, method};
use std::sync::OnceLock;

fn fixture(stripped: bool) -> &'static Fixture {
    static NAMED: OnceLock<Fixture> = OnceLock::new();
    static STRIPPED: OnceLock<Fixture> = OnceLock::new();
    if stripped {
        STRIPPED.get_or_init(|| {
            Fixture::build_stripped(
                "tests/fixtures/source_quality/search",
                "QualitySearchDriver",
            )
        })
    } else {
        NAMED.get_or_init(|| {
            Fixture::build(
                "tests/fixtures/source_quality/search",
                "QualitySearchDriver",
            )
        })
    }
}

#[test]
fn successful_search_work_stays_inside_each_bounded_scan() {
    for stripped in [false, true] {
        let source = fixture(stripped).source("QualitySearch");
        for (name, loops) in [("fallbackSearch", 2), ("chainedFallback", 3)] {
            let body = method(source, name);
            assert_eq!(compact(body).matches("for(").count(), loops, "{body}");
            assert_eq!(compact(body).matches(":{").count(), 1, "{body}");
            assert_eq!(compact(body).matches("choose(").count(), loops, "{body}");
            assert_absent(body, "while (true)");
            assert_absent(body, "; true;");
        }
    }
}

#[test]
fn fallback_exhaustion_keeps_the_enclosing_iteration_continuation() {
    for stripped in [false, true] {
        let body = method(fixture(stripped).source("QualitySearch"), "nestedFallback");
        assert_eq!(compact(body).matches("for(").count(), 3, "{body}");
        assert_eq!(compact(body).matches(":{").count(), 1, "{body}");
        assert_absent(body, "while (true)");
        assert_absent(body, "; true;");
    }
}

#[test]
fn finite_search_decisions_expose_their_one_evaluation() {
    for stripped in [false, true] {
        for name in ["oneShotChoice", "exhaustionEffects", "wrappedChoice"] {
            let body = method(fixture(stripped).source("QualitySearch"), name);
            assert_no_word(body, "int");
            for syntax in ["for (", "while (", "continue;", "break;"] {
                assert_absent(body, syntax);
            }
        }
    }
}

#[test]
fn private_search_result_feeds_its_immediate_consumer_without_a_flag() {
    for stripped in [false, true] {
        for (name, probes) in [("oneShotChoice", 3), ("wrappedChoice", 1)] {
            let body = method(fixture(stripped).source("QualitySearch"), name);
            assert_no_word(body, "boolean");
            assert_eq!(compact(body).matches("if(").count(), 1, "{body}");
            assert_eq!(compact(body).matches("probe(").count(), probes, "{body}");
        }
    }
}

#[test]
fn observed_repeating_and_protected_control_keeps_its_loop() {
    for name in [
        "observedCounter",
        "twoTrips",
        "protectedRepeat",
        "effectfulBound",
    ] {
        let body = method(fixture(false).source("QualitySearch"), name);
        assert!(body.contains("for (") || body.contains("while ("), "{body}");
    }
}
