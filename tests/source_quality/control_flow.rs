//! Readable collection scans, parser loops and command dispatch, not just compilable CFGs.
//! Each cached fixture also checks its complete runtime trace after modern and Java 1.3 recompilation.
use super::support::{
    Fixture, assert_absent, assert_contains as assert_syntax, assert_no_word, compact, method,
};
use std::sync::OnceLock;

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        Fixture::build(
            "tests/fixtures/source_quality/control",
            "QualityControlDriver",
        )
    })
}

fn body(name: &str) -> &'static str {
    method(fixture().source("QualityControl"), name)
}

fn stripped_body(name: &str) -> &'static str {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    let fixture = FIXTURE.get_or_init(|| {
        Fixture::build_stripped(
            "tests/fixtures/source_quality/control",
            "QualityControlDriver",
        )
    });
    method(fixture.source("QualityControl"), name)
}

fn occurrences(body: &str, snippet: &str) -> usize {
    compact(body).matches(&compact(snippet)).count()
}

fn assert_contains(body: &str, snippet: &str) {
    // A self-class qualifier on these fixture calls is optional. These checks
    // protect statement ownership, not the emitter's choice to qualify mark().
    assert_syntax(&body.replace("QualityControl.", ""), snippet);
}

// These loops have a natural condition and update; neither dispatch nor explicit
// exits are needed. Apply this only to those individual methods, not whole sources.
fn bounded_loop(body: &str) {
    assert_contains(body, "for (");
    for lowering in ["while (true)", "switch (", "break;", "continue;", ": {"] {
        assert_absent(body, lowering);
    }
}

fn directly_nested_loop(body: &str) {
    let (_, outer_body) = body.split_once('{').unwrap();
    assert!(
        compact(outer_body).starts_with("for("),
        "the outer loop must contain the inner loop:\n{body}"
    );
}

#[test]
fn stride_update_stays_in_the_counted_loop_header() {
    let body = body("strideSum");
    bounded_loop(body);
    let header = body.split('{').next().unwrap();
    assert_contains(header, "+= 2)");
    assert_contains(header, "<");
    assert_eq!(occurrences(body, "+= 2"), 1, "{body}");
    assert_eq!(occurrences(body, "for ("), 1, "{body}");
}

#[test]
fn reverse_iteration_keeps_the_lower_bound_and_decrement_in_its_header() {
    let body = body("reverseSum");
    bounded_loop(body);
    let header = body.split('{').next().unwrap();
    assert_contains(header, ".length - 1");
    assert_contains(header, ">= 0;");
    assert_contains(header, "--)");
    assert_eq!(occurrences(body, "--"), 1, "{body}");
}

#[test]
fn strided_countdown_exposes_one_test_and_one_update() {
    let body = body("countDownByTwo");
    bounded_loop(body);
    let header = body.split('{').next().unwrap();
    assert_contains(header, "> 0;");
    assert_contains(header, "-= 2)");
    assert_eq!(occurrences(body, "mark("), 1, "{body}");
    assert_no_word(body, "int");
}

#[test]
fn delimiter_scan_combines_the_bound_and_character_test() {
    let body = body("scanSeparator");
    bounded_loop(body);
    let header = body.split('{').next().unwrap();
    assert_contains(header, ".length() &&");
    assert_contains(header, ".charAt(");
    assert_contains(header, "!= ':';");
    assert_contains(header, "++)");
    assert_absent(body, "if (");
}

#[test]
fn reverse_search_has_one_bounded_predicate_without_a_found_flag() {
    let body = body("reverseFind");
    bounded_loop(body);
    let header = body.split('{').next().unwrap();
    assert_contains(header, ".length - 1;");
    assert_contains(header, ">= 0 &&");
    assert_contains(header, "] !=");
    assert_contains(header, "--)");
    assert_no_word(body, "boolean");
    assert_absent(body, "if (");
    assert_eq!(occurrences(body, "return "), 1, "{body}");
}

#[test]
fn decimal_reduction_is_a_post_test_loop_with_compound_division() {
    let body = body("decimalDigits");
    assert_contains(body, "do {");
    assert_contains(body, "} while (");
    assert_contains(body, "/= 10;");
    assert_contains(body, "!= 0);");
    assert_eq!(occurrences(body, "while ("), 1, "{body}");
    assert_absent(body, "if (");
    assert_absent(body, "break;");
}

#[test]
fn mask_reduction_retains_effects_inside_the_post_test_loop() {
    let body = body("shiftUntilClear");
    assert_contains(body, "do { mark(");
    assert_contains(body, ">>>= 1;");
    assert_contains(body, "} while ((");
    assert_contains(body, "&");
    assert_contains(body, ") != 0);");
    assert_eq!(occurrences(body, "mark("), 1, "{body}");
    assert_absent(body, "break;");
    assert_absent(body, "if (");
}

#[test]
fn ragged_matrix_traversal_remains_two_nested_counted_loops() {
    let body = body("matrixSum");
    bounded_loop(body);
    directly_nested_loop(body);
    assert_eq!(occurrences(body, "for ("), 2, "{body}");
    assert_eq!(occurrences(body, ".length;"), 2, "{body}");
    assert_eq!(occurrences(body, "++)"), 2, "{body}");
    assert_eq!(occurrences(body, "+="), 1, "{body}");
    assert_absent(body, "if (");
}

#[test]
fn per_row_sentinel_is_folded_into_only_the_inner_loop_condition() {
    let body = body("prefixRows");
    bounded_loop(body);
    directly_nested_loop(body);
    assert_eq!(occurrences(body, "for ("), 2, "{body}");
    assert_eq!(occurrences(body, "&&"), 1, "{body}");
    assert_contains(body, "] >= 0;");
    assert_eq!(occurrences(body, "mark("), 1, "{body}");
    assert_contains(body, "} mark(");
    assert_absent(body, "if (");
}

#[test]
fn rejecting_guards_stay_flat_before_the_useful_branch() {
    let body = body("guardedLength");
    assert_contains(body, "== null) { return -1; }");
    assert_contains(body, "< 0) { return -2; }");
    assert_eq!(occurrences(body, "if ("), 3, "{body}");
    assert_no_word(body, "else");
    assert_no_word(body, "boolean");
    assert_eq!(occurrences(body, "mark("), 1, "{body}");
}

#[test]
fn invalid_range_uses_one_compound_throwing_guard() {
    let body = body("checkedRange");
    assert_contains(body, "if (");
    assert_contains(body, "< 0 ||");
    assert_contains(body, "throw new IllegalArgumentException();");
    assert_eq!(occurrences(body, "if ("), 1, "{body}");
    assert_no_word(body, "else");
    assert_eq!(occurrences(body, "mark("), 2, "{body}");
}

#[test]
fn classification_uses_an_else_if_chain_and_one_shared_suffix() {
    let body = body("classify");
    assert_contains(body, "} else if (");
    assert_contains(body, "} else {");
    assert_eq!(occurrences(body, "if ("), 2, "{body}");
    assert_eq!(occurrences(body, "mark(40)"), 1, "{body}");
    assert_contains(body, "} mark(40);");
    assert_absent(body, "switch (");
    assert_absent(body, "return;");
}

#[test]
fn nested_decision_keeps_inner_and_outer_continuations_shared() {
    let body = body("nestedDecision");
    assert_eq!(occurrences(body, "if ("), 2, "{body}");
    assert_eq!(occurrences(body, "else {"), 2, "{body}");
    assert_contains(body, "} mark(4);");
    assert_contains(body, "} mark(6);");
    for value in [1, 2, 3, 4, 5, 6] {
        assert_eq!(occurrences(body, &format!("mark({value})")), 1, "{body}");
    }
    assert_absent(body, "return;");
    assert_absent(body, "switch (");
}

#[test]
fn dense_dispatch_returns_directly_from_each_case() {
    let body = body("denseCommand");
    assert_eq!(occurrences(body, "switch ("), 1, "{body}");
    for (case, result) in [(1, 10), (2, 20), (3, 30)] {
        assert_contains(body, &format!("case {case}: return {result};"));
    }
    assert_contains(body, "default: return -1;");
    assert_no_word(body, "int");
    assert_no_word(body, "break");
    assert_absent(body, "if (");
}

#[test]
fn sparse_dispatch_keeps_noncontiguous_case_labels() {
    let body = body("sparseCommand");
    assert_eq!(occurrences(body, "switch ("), 1, "{body}");
    for (case, result) in [(-100, 7), (9, 11), (1000, 13)] {
        assert_contains(body, &format!("case {case}: return {result};"));
    }
    assert_contains(body, "default: return -1;");
    assert_absent(body, "if (");
    assert_no_word(body, "int");
}

#[test]
fn equivalent_commands_share_stacked_labels_and_one_case_body() {
    let body = body("groupedCommands");
    assert_contains(body, "case 2: case 5:");
    assert_contains(body, "mark(10); mark(11); break;");
    assert_eq!(occurrences(body, "mark(10)"), 1, "{body}");
    assert_eq!(occurrences(body, "mark(11)"), 1, "{body}");
    assert_eq!(occurrences(body, "mark(40)"), 1, "{body}");
    assert_contains(body, "} mark(40);");
    assert_absent(body, "if (");
}

#[test]
fn effectful_fallthrough_keeps_the_effect_between_adjacent_labels() {
    let body = body("fallthroughCommand");
    assert_contains(body, "case 0: mark(10); case 1:");
    assert_contains(body, "case 1: mark(11); break;");
    assert_eq!(occurrences(body, "mark(10)"), 1, "{body}");
    assert_eq!(occurrences(body, "mark(11)"), 1, "{body}");
    assert_eq!(occurrences(body, "mark(40)"), 1, "{body}");
    assert_contains(body, "} mark(40);");
    assert_absent(body, "if (");
}

#[test]
fn switch_arms_keep_their_counted_post_test_and_conditional_bodies() {
    let body = body("loopCommand");
    assert_contains(body, "case 1: for (");
    assert_contains(body, "case 2: do {");
    assert_contains(body, "default: if (");
    assert_eq!(occurrences(body, "switch ("), 1, "{body}");
    assert_eq!(occurrences(body, "for ("), 1, "{body}");
    assert_eq!(occurrences(body, "} while ("), 1, "{body}");
    assert_eq!(occurrences(body, "mark(40)"), 1, "{body}");
    assert_contains(body, "} mark(40);");
    assert_absent(body, "while (true)");
}

#[test]
fn dispatch_loop_owns_switch_breaks_and_the_per_iteration_suffix() {
    let body = body("dispatchLoop");
    assert_eq!(occurrences(body, "for ("), 1, "{body}");
    assert_eq!(occurrences(body, "switch ("), 1, "{body}");
    let (_, loop_body) = body.split_once('{').unwrap();
    assert!(compact(loop_body).starts_with("switch("), "{body}");
    assert_contains(body, "case 0: mark(10); break;");
    assert_contains(body, "case 1: mark(20); break;");
    assert_contains(body, "} mark(40); }");
    assert_eq!(occurrences(body, "mark(40)"), 1, "{body}");
    assert_absent(body, "continue;");
    assert_absent(body, "while (true)");
}

#[test]
fn guard_precedes_switch_with_one_shared_return_continuation() {
    let body = body("guardedSwitch");
    assert_contains(body, ") { return -1; } mark(1); switch (");
    assert_contains(body, "} mark(40); return 0;");
    assert_eq!(occurrences(body, "return "), 2, "{body}");
    assert_eq!(occurrences(body, "mark(40)"), 1, "{body}");
    assert_no_word(body, "else");
    assert_no_word(body, "boolean");
}

#[test]
fn stripped_debug_scan_still_has_one_combined_predicate() {
    let body = stripped_body("scanSeparator");
    bounded_loop(body);
    let header = body.split('{').next().unwrap();
    assert_contains(header, ".length() &&");
    assert_contains(header, ".charAt(");
    assert_contains(header, "!= ':';");
    assert_contains(header, "++)");
    assert_absent(body, "if (");
}

#[test]
fn stripped_debug_matrix_loops_keep_nesting_without_control_scaffolding() {
    let body = stripped_body("matrixSum");
    bounded_loop(body);
    directly_nested_loop(body);
    assert_eq!(occurrences(body, "for ("), 2, "{body}");
    assert_eq!(occurrences(body, ".length;"), 2, "{body}");
    assert_eq!(occurrences(body, "++)"), 2, "{body}");
    assert_absent(body, "if (");
}

#[test]
fn stripped_debug_dispatch_preserves_fallthrough_and_shared_continuation() {
    let body = stripped_body("fallthroughCommand");
    assert_contains(body, "case 0: mark(10); case 1:");
    assert_contains(body, "case 1: mark(11); break;");
    assert_contains(body, "} mark(40);");
    assert_eq!(occurrences(body, "switch ("), 1, "{body}");
    assert_eq!(occurrences(body, "mark(40)"), 1, "{body}");
    assert_absent(body, "if (");
}
