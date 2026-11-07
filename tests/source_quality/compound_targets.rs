//! A duplicated JVM lvalue has one evaluated owner in Java compound syntax.
use super::support::{Fixture, assert_absent, assert_contains, compact, method};
use std::sync::OnceLock;

fn fixtures() -> &'static [Fixture; 2] {
    static FIXTURES: OnceLock<[Fixture; 2]> = OnceLock::new();
    FIXTURES.get_or_init(|| {
        [
            Fixture::build(
                "tests/fixtures/source_quality/compound_targets",
                "CompoundTargets",
            ),
            Fixture::build_stripped(
                "tests/fixtures/source_quality/compound_targets",
                "CompoundTargets",
            ),
        ]
    })
}

#[test]
fn field_arrays_and_computed_indexes_stay_in_the_compound_target() {
    for fixture in fixtures() {
        for name in ["direct", "computed", "fromArray"] {
            let body = method(fixture.source("CompoundTargets"), name);
            assert_contains(body, "this.bytes[");
            assert_contains(body, "] ^=");
            assert_eq!(body.matches(';').count(), 1, "{name}: {body}");
            assert_absent(body, "byte[]");
            if name == "computed" {
                assert_contains(body, "* 2]");
            }
        }
    }
}

#[test]
fn receiver_index_and_rhs_effects_have_one_direct_owner() {
    for fixture in fixtures() {
        for (name, snippet) in [
            (
                "receiverTarget",
                "receiver().bytes[CompoundTargets.index()] ^= CompoundTargets.rhs()",
            ),
            ("fieldTarget", "receiver().count *= CompoundTargets.rhs()"),
            (
                "publishedTarget",
                "this.published[CompoundTargets.index()] -= CompoundTargets.rhs()",
            ),
            (
                "replacedTarget",
                "this.bytes[CompoundTargets.index()] ^= this.replace()",
            ),
            ("replacedField", "this.count *= this.replaceCount()"),
        ] {
            let body = method(fixture.source("CompoundTargets"), name);
            assert_contains(body, snippet);
            assert_eq!(body.matches(';').count(), 1, "{name}: {body}");
        }
    }
}

#[test]
fn shared_targets_stay_captured_and_numeric_widths_stay_explicit() {
    for fixture in fixtures() {
        let source = fixture.source("CompoundTargets");
        let shared = method(source, "sharedTarget");
        assert_contains(shared, "byte[]");
        assert_contains(shared, "= this.bytes;");
        assert_eq!(compact(shared).matches("this.bytes").count(), 1, "{shared}");
        let numeric = method(source, "numeric");
        for snippet in [
            "this.bytes[0] +=",
            "this.bytes[1] *= 257",
            "this.numbers[0] += 31",
            "this.numbers[1] >>>=",
            "this.count /= 3",
        ] {
            assert_contains(numeric, snippet);
        }
        assert_eq!(numeric.matches(';').count(), 5, "{numeric}");
    }
}

#[test]
fn finally_copies_share_the_same_owned_target() {
    for fixture in fixtures() {
        let body = method(fixture.source("CompoundTargets"), "finalTarget");
        assert_contains(body, "finally {");
        assert_contains(
            body,
            "this.bytes[CompoundTargets.index()] ^= CompoundTargets.rhs()",
        );
        assert_eq!(body.matches("^=").count(), 1, "{body}");
    }
}
