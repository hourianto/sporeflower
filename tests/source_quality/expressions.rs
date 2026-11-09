//! Readable expression composition, checked independently from runtime equivalence.
use super::support::{
    Fixture, assert_absent, assert_contains, assert_no_word, compact, method, word_count,
};
use std::sync::OnceLock;

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        Fixture::build(
            "tests/fixtures/source_quality/expressions",
            "QualityExpressionDriver",
        )
    })
}

fn body(name: &str) -> &'static str {
    method(fixture().source("QualityExpressions"), name)
}

fn numeric_body(name: &str) -> &'static str {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    let fixture = FIXTURE.get_or_init(|| {
        Fixture::build_stripped(
            "tests/fixtures/source_quality/expressions",
            "QualityExpressionDriver",
        )
    });
    method(fixture.source("QualityExpressions"), name)
}

#[test]
fn null_returns_use_their_declared_context_but_call_arguments_keep_their_overload() {
    for name in ["nullArray", "nullString"] {
        assert_contains(body(name), "return null;");
        assert_contains(numeric_body(name), "return null;");
    }
    assert_contains(numeric_body("nullOverload"), "nullKind((Object) null)");
}

#[test]
fn numeric_assignment_and_return_contexts_discharge_representable_literal_casts() {
    for name in [
        "constantReturn",
        "switchLiteral",
        "constantStores",
        "byteChoice",
    ] {
        assert_absent(numeric_body(name), "(byte)");
    }
    for name in [
        "characterChoice",
        "mixedChoice",
        "promotedNarrow",
        "nestedNarrow",
    ] {
        assert_absent(numeric_body(name), "(int)");
    }
    assert_contains(numeric_body("promotedNarrow"), "(short)");
    assert_contains(numeric_body("nestedNarrow"), "(short)");
}

#[test]
fn numeric_contexts_retain_truncation_overload_character_and_rounding_conversions() {
    assert_contains(numeric_body("narrowedThenWidened"), "(byte)");
    assert_contains(numeric_body("widenedOverload"), "numericKind((int)");
    assert_contains(numeric_body("characterAsNumber"), "(int)");
    assert_contains(numeric_body("narrowAsCharacter"), "(char)");
    assert_contains(numeric_body("roundedWidening"), "(float)");
}

/// These examples need one source expression, without branch or temporary scaffolding.
/// This checks the statement shape without freezing parameter names or a whole return.
fn returned_expression(name: &str) -> &'static str {
    let body = body(name);
    let text = compact(body);
    assert!(text.starts_with("return"), "{name}:\n{body}");
    assert_eq!(text.matches(';').count(), 1, "{name}:\n{body}");
    assert_absent(body, "if (");
    body
}

#[test]
fn null_bounds_and_character_checks_form_one_guarded_boolean_return() {
    let body = returned_expression("pathCharacter");
    assert_contains(body, "!= null &&");
    assert_contains(body, ">= 0 &&");
    assert_contains(body, ".length() && (");
    assert_contains(body, "== '/' ||");
    assert_contains(body, "== '?')");
    assert_eq!(compact(body).matches("charAt(").count(), 2, "{body}");
    assert_eq!(body.matches("&&").count(), 3, "{body}");
}

#[test]
fn lazy_predicates_keep_conjunction_inside_disjunction() {
    let body = returned_expression("lazyPolicy");
    let text = compact(body);
    assert_eq!(text.matches("probe(").count(), 2, "{body}");
    assert_eq!(text.matches("&&").count(), 1, "{body}");
    assert_eq!(text.matches("||").count(), 1, "{body}");
    assert!(text.find("&&") < text.find("probe("), "{body}");
    assert!(text.find("probe(") < text.find("||"), "{body}");
    assert!(text.find("||") < text.rfind("probe("), "{body}");
    assert_absent(body, "?");
}

#[test]
fn eager_boolean_operations_remain_direct_bitwise_expressions() {
    let body = returned_expression("eagerPolicy");
    assert_eq!(compact(body).matches("probe(").count(), 3, "{body}");
    assert_eq!(body.matches('&').count(), 1, "{body}");
    assert_eq!(body.matches('^').count(), 1, "{body}");
    assert_contains(body, "probe(3)");
    assert_absent(body, "||");
    assert_absent(body, "?");
}

#[test]
fn conditional_boolean_arms_retain_their_lazy_operators() {
    let body = returned_expression("selectedPolicy");
    assert_eq!(body.matches('?').count(), 1, "{body}");
    assert_eq!(body.matches(':').count(), 1, "{body}");
    assert_eq!(compact(body).matches("probe(").count(), 4, "{body}");
    assert_eq!(body.matches("&&").count(), 1, "{body}");
    assert_eq!(body.matches("||").count(), 1, "{body}");
    assert_no_word(body, "true");
    assert_no_word(body, "false");
}

#[test]
fn nested_numeric_selection_stays_a_single_return_expression() {
    let body = returned_expression("clampWithEffects");
    assert_eq!(body.matches('?').count(), 2, "{body}");
    assert_eq!(body.matches(':').count(), 2, "{body}");
    assert_eq!(compact(body).matches("number(").count(), 3, "{body}");
    assert_contains(body, "number(0)");
    assert_contains(body, "number(10)");
}

#[test]
fn buffer_chain_becomes_concatenation_with_an_inline_conditional() {
    let body = returned_expression("statusLine");
    assert_contains(body, "+ ':' +");
    assert_contains(body, "number(1)");
    assert_contains(body, "\"/ready\"");
    assert_contains(body, "\"/waiting\"");
    assert_contains(body, "+ '!'");
    assert_eq!(body.matches('+').count(), 4, "{body}");
    assert_eq!(body.matches('?').count(), 1, "{body}");
    for lowering in ["StringBuffer", "append(", "toString("] {
        assert_absent(body, lowering);
    }
}

#[test]
fn numeric_buffer_seed_preserves_string_promotion_and_long_literals() {
    let body = returned_expression("storageLabel");
    assert_contains(body, "/ 1024L + \"KB+\" +");
    assert_contains(body, "% 1024L + 'B'");
    assert_eq!(body.matches("1024L").count(), 2, "{body}");
    for lowering in ["StringBuffer", "String.valueOf", "append(", "(long)"] {
        assert_absent(body, lowering);
    }
}

#[test]
fn character_ranges_and_escaped_delimiters_use_character_literals() {
    let body = returned_expression("characterLiterals");
    for spelling in [">= 'A' &&", "<= 'Z') ||", "== '\\n' ||", "== '\\\\'"] {
        assert_contains(body, spelling);
    }
    for numeric_spelling in [">= 65", "<= 90", "== 10", "== 92", "(char)"] {
        assert_absent(body, numeric_spelling);
    }
}

#[test]
fn factored_character_ranges_keep_direct_ordered_comparisons() {
    for body in [
        body("normalizedCharacter"),
        numeric_body("normalizedCharacter"),
    ] {
        assert_contains(body, ">= 'A' &&");
        assert_contains(body, "<= 'F'");
        assert_absent(body, "!(");
    }
}

#[test]
fn signed_constants_and_shift_precedence_read_as_ordinary_arithmetic() {
    let body = returned_expression("integerArithmetic");
    assert_contains(body, "- 7) * (");
    assert_contains(body, "+ 3) + (");
    assert_contains(body, ">>> 5)");
    assert_absent(body, "+ -7");
    assert_absent(body, "- -3");
    assert_absent(body, "(int)");
}

#[test]
fn widening_product_keeps_one_cast_at_the_arithmetic_boundary() {
    let body = returned_expression("wideProduct");
    assert_contains(body, "return (long) (");
    assert_contains(body, "+ 1) * (");
    assert_contains(body, "- 1) +");
    assert_eq!(compact(body).matches("(long)").count(), 1, "{body}");
    assert_absent(body, "(int)");
}

#[test]
fn floating_division_converts_the_sum_once_before_dividing() {
    let body = returned_expression("ratio");
    assert_contains(body, "return (double) (");
    assert_contains(body, "+ 1) /");
    assert_eq!(compact(body).matches("(double)").count(), 1, "{body}");
    assert_absent(body, "(int)");
}

#[test]
fn reference_return_context_needs_no_casts_on_conditional_arms() {
    let body = returned_expression("referenceChoice");
    assert_eq!(body.matches('?').count(), 1, "{body}");
    assert_eq!(body.matches(':').count(), 1, "{body}");
    assert_absent(body, "(Object)");
    assert_absent(body, "(String)");
    assert_absent(body, "=");
}

#[test]
fn overload_selection_keeps_only_the_required_reference_casts() {
    let body = returned_expression("requiredOverload");
    assert_eq!(compact(body).matches("kind((Object)").count(), 2, "{body}");
    assert_contains(body, "kind((Object) null)");
    assert_eq!(body.matches('+').count(), 1, "{body}");
    assert_absent(body, "(String)");
}

#[test]
fn array_compound_updates_do_not_expand_load_modify_store_sequences() {
    let body = body("arrayCompound");
    assert_contains(body, "] +=");
    assert_contains(body, "] ^=");
    assert_contains(body, "number(5)");
    assert_contains(body, "number(3)");
    assert_eq!(body.matches(';').count(), 3, "{body}");
    assert_eq!(body.matches('[').count(), 3, "{body}");
    assert_absent(body, "if (");
}

#[test]
fn field_compound_updates_preserve_the_direct_receiver() {
    let body = body("fieldCompound");
    assert_contains(body, ".count -=");
    assert_contains(body, "number(5)");
    assert_contains(body, ".count |= 1");
    assert_eq!(compact(body).matches(".count").count(), 3, "{body}");
    assert_eq!(body.matches(';').count(), 3, "{body}");
    assert_absent(body, "if (");
}

#[test]
fn prefix_and_postfix_updates_compose_with_effectful_receivers() {
    let body = returned_expression("prefixAndPostfix");
    assert_contains(body, "index()]++ + ++");
    assert_contains(body, "receiver().count");
    assert_eq!(compact(body).matches("index()").count(), 1, "{body}");
    assert_eq!(compact(body).matches("receiver()").count(), 1, "{body}");
    assert_eq!(body.matches("++").count(), 2, "{body}");
    assert_absent(body, "=");
}

#[test]
fn packed_byte_writes_fold_index_increments_into_each_store() {
    let body = body("writePacket");
    assert_eq!(compact(body).matches("++]=(byte)").count(), 4, "{body}");
    for shift in [">>> 24", ">>> 16", ">>> 8"] {
        assert_contains(body, shift);
    }
    assert_eq!(body.matches(';').count(), 5, "{body}");
    assert_absent(body, "+= 1");
}

#[test]
fn effectful_array_elements_stay_inside_one_initializer() {
    let body = returned_expression("effectfulInitializer");
    assert_contains(body, "return new int[] {");
    for call in ["number(1)", "number(2)", "number(3)", "number(4)"] {
        assert_eq!(compact(body).matches(call).count(), 1, "{body}");
    }
    assert_contains(body, "number(2) +");
    assert_eq!(body.matches(',').count(), 2, "{body}");
    assert_absent(body, "] =");
}

#[test]
fn jagged_arrays_keep_rows_in_one_nested_initializer() {
    let body = returned_expression("nestedInitializer");
    assert_contains(body, "return new int[][] { {");
    assert_contains(body, "+ 1 }, {");
    assert_contains(body, "+ 2 },");
    let text = compact(body);
    assert!(
        text.contains(",newint[0]}") || text.contains(",{}}"),
        "{body}"
    );
    assert_absent(body, "] =");
}

#[test]
fn dominated_null_facts_remove_redundant_checks_after_effects() {
    let body = body("guardedLength");
    assert_eq!(compact(body).matches("if(").count(), 1, "{body}");
    assert_eq!(word_count(body, "null"), 1, "{body}");
    assert_contains(body, "return -1;");
    assert_contains(body, "number(6)");
    assert_contains(body, ".length()");
    assert_absent(body, "return -2");
    assert_eq!(body.matches(';').count(), 3, "{body}");
    let text = compact(body);
    assert!(text.find("number(6)") < text.find(".length()"), "{body}");
}
