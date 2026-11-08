//! Final expressions share the same storage and evaluation-order rules at each destination.
use super::support::{Fixture, assert_absent, assert_contains, compact, method, word_count};
use std::sync::OnceLock;

fn fixtures() -> &'static [Fixture; 2] {
    static FIXTURES: OnceLock<[Fixture; 2]> = OnceLock::new();
    FIXTURES.get_or_init(|| {
        [
            Fixture::build(
                "tests/fixtures/source_quality/storage_expressions",
                "QualityStorageDriver",
            ),
            Fixture::build_stripped(
                "tests/fixtures/source_quality/storage_expressions",
                "QualityStorageDriver",
            ),
        ]
    })
}

// Names from real debug metadata may be useful. Elimination contracts concern
// generated carriers; both metadata variants still run the full runtime batch.
fn stripped_source() -> &'static str {
    fixtures()[1].source("QualityStorageExpressions")
}

#[test]
fn field_choice_inlines_its_result_and_retains_the_old_value_snapshot() {
    let body = method(stripped_source(), "fieldChoice");
    assert_eq!(word_count(body, "boolean"), 1, "{body}");
    let store = body
        .lines()
        .find(|line| line.contains(".enabled ="))
        .unwrap();
    assert!(
        store.contains('?'),
        "choice should belong to field store:\n{body}"
    );
    assert_eq!(compact(body).matches(".enabled").count(), 2, "{body}");
    assert_eq!(compact(body).matches("mark(").count(), 1, "{body}");
}

#[test]
fn one_use_choices_compose_with_array_and_call_destinations() {
    let source = stripped_source();
    for name in ["arrayChoice", "argumentChoice"] {
        let body = method(source, name);
        assert_eq!(word_count(body, "int"), 0, "{name}:\n{body}");
        assert_eq!(body.matches(';').count(), 1, "{name}:\n{body}");
        assert_eq!(body.matches('?').count(), 1, "{name}:\n{body}");
    }
    let body = method(source, "nestedArrayChoice");
    // The loaded value participates in both the predicate and the result.
    // The single-use alpha choice belongs inside the shift/store expression.
    assert_eq!(word_count(body, "int"), 1, "{body}");
    assert_eq!(body.matches(';').count(), 2, "{body}");
    assert_contains(body, "<< 8");
    assert_eq!(body.matches('?').count(), 1, "{body}");
}

#[test]
fn retained_choice_declaration_stays_at_its_first_value_after_prior_effects() {
    for fixture in fixtures() {
        let body = method(
            fixture.source("QualityStorageExpressions"),
            "retainedChoiceAfterEffect",
        );
        let text = compact(body);
        assert!(
            text.starts_with("QualityStorageExpressions.mark(7);int"),
            "{body}"
        );
        assert_eq!(word_count(body, "int"), 1, "{body}");
        assert_eq!(body.matches(';').count(), 3, "{body}");
        assert_eq!(body.matches('?').count(), 1, "{body}");
        assert_eq!(text.matches("mark(").count(), 3, "{body}");
    }
}

#[test]
fn branch_boolean_results_return_directly_without_result_storage() {
    let body = method(stripped_source(), "branchReturn");
    assert_eq!(word_count(body, "boolean"), 0, "{body}");
    assert_eq!(compact(body).matches("probe(").count(), 2, "{body}");
    assert_contains(body, "&&");
}

#[test]
fn shared_result_switch_arms_return_their_values_directly() {
    let body = method(stripped_source(), "sharedSwitchResult");
    assert_eq!(word_count(body, "switch"), 1, "{body}");
    // The final case deliberately falls through to a default shared with the
    // unmatched selector. Only the four terminal cases have private results.
    for value in 1..=4 {
        assert_contains(
            body,
            &format!("return QualityStorageExpressions.mark({value});"),
        );
    }
}

#[test]
fn redundant_numeric_storage_keeps_the_widening_at_the_overload_boundary() {
    let source = stripped_source();
    let body = method(source, "widenedCopy");
    assert_eq!(body.matches(';').count(), 1, "{body}");
    assert_contains(body, "kind((long)");
    assert_eq!(compact(body).matches("mark(").count(), 1, "{body}");
    // The runtime batch distinguishes byte/int, int/long, and String/Object
    // overloads even when the assignment that established the type vanishes.
}

#[test]
fn null_arms_keep_the_non_null_reference_type_without_an_outer_cast() {
    let source = stripped_source();
    for name in ["nullableString", "mirroredNullableString"] {
        let body = method(source, name);
        assert!(compact(body).starts_with("return"), "{body}");
        assert_eq!(body.matches(';').count(), 1, "{body}");
        assert_eq!(body.matches('?').count(), 1, "{body}");
        assert_absent(body, "(String)");
    }
    // The broader Object view is significant when it selects an overload.
    assert_contains(method(source, "referenceChoice"), "kind((Object)");
}

#[test]
fn integer_identity_and_wrapping_cancellation_leave_direct_values() {
    let source = stripped_source();
    for name in ["integerIdentity", "integerCancellation", "longCancellation"] {
        let body = method(source, name);
        assert_eq!(body.matches(';').count(), 1, "{body}");
        for operation in ["+", "-", "*"] {
            assert_absent(body, operation);
        }
    }
}

#[test]
fn cleanup_monitor_and_prior_operand_observations_survive_simplification() {
    for fixture in fixtures() {
        let source = fixture.source("QualityStorageExpressions");
        assert_contains(method(source, "returnedBeforeFinally"), "finally {");
        let monitor = method(source, "monitorCapture");
        assert_contains(monitor, "synchronized (");
        assert_eq!(compact(monitor).matches("lock()").count(), 1, "{monitor}");
        assert_eq!(
            compact(monitor).matches("ownsLock(").count(),
            2,
            "{monitor}"
        );
        let direct = method(source, "directMonitor");
        assert_contains(direct, "synchronized (");
        assert_eq!(compact(direct).matches("lock()").count(), 1, "{direct}");
        // The shared runtime batch checks every throwing marker and the stored
        // state after it, so moving a result past a receiver, index, earlier
        // argument, write, or cleanup is observable independently of its spelling.
    }
}

#[test]
fn handler_exits_keep_storage_observed_by_their_continuations() {
    for fixture in fixtures() {
        let source = fixture.source("QualityStorageExpressions");
        for name in [
            "handlerContinuation",
            "pureHandlerContinuation",
            "handlerBreak",
            "handlerContinue",
            "handlerCleanup",
        ] {
            let body = method(source, name);
            assert_contains(body, "catch (RuntimeException");
            assert_eq!(
                compact(body).matches("observeHandlerValue(").count(),
                1,
                "{name}:\n{body}"
            );
        }
        let breaking = method(source, "handlerBreak");
        assert_eq!(word_count(breaking, "break"), 1, "{breaking}");
        let continuing = method(source, "handlerContinue");
        assert_eq!(word_count(continuing, "continue"), 1, "{continuing}");
        assert_contains(method(source, "handlerCleanup"), "finally {");
        // The driver asserts the stored value after each actual throwing marker,
        // including empty catch fallthrough, a catch break, a catch continue to
        // the next iteration's observer, and cleanup before its replacement.
    }
}

#[test]
fn unobserved_handler_results_still_inline_into_their_only_argument() {
    let source = stripped_source();
    let immediate = method(source, "immediateHandlerArgument");
    assert_eq!(word_count(immediate, "int"), 0, "{immediate}");
    assert_contains(
        immediate,
        "observeHandlerValue(QualityStorageExpressions.mark(1));",
    );
    let pure = method(source, "pureHandlerArgument");
    assert_eq!(word_count(pure, "int"), 0, "{pure}");
    assert_eq!(
        compact(pure).matches("observeHandlerValue(").count(),
        1,
        "{pure}"
    );
}
