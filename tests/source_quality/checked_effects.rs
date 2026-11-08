//! Checked catch validity follows surviving source operations after rewrites.
use super::support::{Fixture, assert_absent, assert_contains, compact, method};
use std::sync::OnceLock;

fn fixtures() -> &'static [Fixture; 2] {
    static FIXTURES: OnceLock<[Fixture; 2]> = OnceLock::new();
    FIXTURES.get_or_init(|| {
        let path = "tests/fixtures/source_quality/checked_effects";
        [
            Fixture::build(path, "QualityCheckedEffects"),
            Fixture::build_stripped(path, "QualityCheckedEffects"),
        ]
    })
}

#[test]
fn removed_checked_operation_keeps_a_valid_owned_catch() {
    for fixture in fixtures() {
        let body = method(fixture.source("QualityCheckedEffects"), "zeroTrips");
        assert_absent(body, "for (");
        assert_absent(body, "checked(");
        assert_contains(&compact(body), "if(false)");
        assert_contains(body, "throw (IOException) null;");
        assert_eq!(body.matches("mark(1)").count(), 1, "{body}");
    }
}

#[test]
fn inlined_calls_and_constructors_retain_their_intrinsic_contract() {
    for fixture in fixtures() {
        let source = fixture.source("QualityCheckedEffects");
        for name in ["retainedCall", "retainedConstructor"] {
            let body = method(source, name);
            assert_absent(&compact(body), "if(false)");
            assert_contains(body, "catch (IOException");
        }
    }
    let source = fixtures()[1].source("QualityCheckedEffects");
    let call = method(source, "retainedCall");
    assert!(
        call.lines()
            .any(|line| { line.trim().starts_with("return (byte) ") && line.contains("checked(") }),
        "{call}"
    );
    assert_contains(method(source, "retainedConstructor"), "new CheckedBox(");
}
