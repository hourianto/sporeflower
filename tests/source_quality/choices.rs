//! Choice expressions retain storage updates and shared arm-local definitions.
use super::support::{Fixture, assert_absent, assert_contains, method};
use std::sync::OnceLock;

fn fixture(stripped: bool) -> &'static Fixture {
    static DEBUG: OnceLock<Fixture> = OnceLock::new();
    static STRIPPED: OnceLock<Fixture> = OnceLock::new();
    (if stripped { &STRIPPED } else { &DEBUG }).get_or_init(|| {
        let directory = "tests/fixtures/source_quality/choices";
        if stripped {
            Fixture::build_stripped(directory, "QualityChoices")
        } else {
            Fixture::build(directory, "QualityChoices")
        }
    })
}

#[test]
fn updates_and_shared_arm_values_keep_statement_owners() {
    for stripped in [false, true] {
        let source = fixture(stripped).source("QualityChoices");
        for name in ["adjustedIndex", "selectedBuffer", "changedSelector"] {
            let body = method(source, name);
            assert_contains(body, "if (");
            assert_absent(body, "?");
        }
        let body = method(source, "selectedBuffer");
        assert_contains(body, "byte[]");
        assert_absent(body, "consume((");
    }
    let source = fixture(false).source("QualityChoices");
    let adjusted = method(source, "adjustedIndex");
    assert_contains(adjusted, "int result = input; if (enabled) {");
    assert_contains(adjusted, "if (input >= 2 && first) { result = input + 3; }");
    assert_contains(
        adjusted,
        "else if (input >= 4 && second) { result = input + 7; }",
    );
    let mutation = method(source, "changedSelector");
    assert_contains(
        mutation,
        "if (position++ < limit) { result = position; } else { result = limit; }",
    );
    assert_contains(
        mutation,
        "return QualityChoices.observe(result) + position + limit;",
    );
    let buffer = method(source, "selectedBuffer");
    assert_contains(
        buffer,
        "byte[] data = QualityChoices.bytes(input); result = QualityChoices.consume(data, data.length);",
    );
}

#[test]
fn nested_default_decisions_keep_one_binding_and_one_positive_override() {
    for stripped in [false, true] {
        let body = method(fixture(stripped).source("QualityChoices"), "nestedDefault");
        assert_eq!(body.matches("if (").count(), 2, "{body}");
        assert_absent(body, "else if (");
        assert_absent(body, "?");
        assert_eq!(body.matches("&&").count(), 2, "{body}");
    }
    let body = method(fixture(false).source("QualityChoices"), "nestedDefault");
    assert_contains(body, "int result = input;");
    assert_contains(body, "if (first && input >= 0 && second) {");
    assert_contains(body, "result = QualityChoices.observe(input + 3);");
    assert_contains(
        body,
        "QualityChoices.observe(result); return result + input;",
    );
}

#[test]
fn compound_special_case_keeps_a_positive_assignment_guard() {
    let body = method(fixture(false).source("QualityChoices"), "compoundRule");
    assert_contains(
        body,
        "else if (enabled && group == 2 && mode == 0 && target >= 0)",
    );
    assert_contains(body, "range = -1;");
    assert_contains(body, "range = Math.max(first, second);");
    assert_absent(body, "?");
}

#[test]
fn simple_alternatives_and_clamps_remain_expressions() {
    for stripped in [false, true] {
        for name in ["advanceBoundary", "label", "clamp", "calls"] {
            let body = method(fixture(stripped).source("QualityChoices"), name);
            assert_contains(body, "?");
            assert_absent(body, "if (");
        }
    }
    assert_contains(
        method(fixture(false).source("QualityChoices"), "advanceBoundary"),
        "position == -1 ? limit : position + 1",
    );
}

#[test]
fn storage_ladders_preserve_mutation_and_handler_snapshots() {
    for stripped in [false, true] {
        let source = fixture(stripped).source("QualityChoices");
        for name in ["changedDefault", "caughtDefault"] {
            let body = method(source, name);
            assert_eq!(body.matches("else if (").count(), 2, "{name}: {body}");
            assert_absent(body, "?");
        }
    }
    let source = fixture(false).source("QualityChoices");
    let changed = method(source, "changedDefault");
    assert_contains(
        changed,
        "if (first) { result = input; } else if (input++ > 1) { result = input + 2; }",
    );
    assert_contains(
        changed,
        "else if (second) { result = input; } else { result = input - 3; }",
    );
    assert_contains(changed, "return result + input;");
    let caught = method(source, "caughtDefault");
    assert_contains(
        caught,
        "int result = -17; try { if (first) { result = input; }",
    );
    assert_contains(
        caught,
        "else if (QualityChoices.checked(input)) { result = input + 2; }",
    );
    assert_contains(
        caught,
        "catch (IllegalArgumentException e) { QualityChoices.observe(result); } return result + input;",
    );
}

#[test]
fn independent_guarded_diamonds_stay_compact() {
    for stripped in [false, true] {
        let source = fixture(stripped).source("QualityChoices");
        for name in ["guardedChoice", "independentChoices"] {
            let body = method(source, name);
            assert_eq!(body.matches("if (").count(), 1, "{name}: {body}");
            assert_contains(body, "?");
        }
        let narrow = method(source, "narrowAlternative");
        assert_eq!(narrow.matches(" ? ").count(), 1, "{narrow}");
        assert_absent(narrow, "if (");
        assert_absent(narrow, "short ");
        assert_contains(narrow, "[5]");
    }
    let source = fixture(false).source("QualityChoices");
    assert_contains(
        method(source, "guardedChoice"),
        "if (enabled) { result = first < second ? -1 : 1; }",
    );
    assert_contains(method(source, "narrowAlternative"), "input < 0 ?");
}

#[test]
fn guarded_defaults_accept_widening_and_literal_initializers() {
    for stripped in [false, true] {
        let source = fixture(stripped).source("QualityChoices");
        for name in ["widenedDefault", "literalDefault"] {
            let body = method(source, name);
            assert_eq!(body.matches("if (").count(), 1, "{name}: {body}");
            assert_contains(body, "&&");
            assert_absent(body, "else");
            assert_absent(body, "?");
        }
    }
    let source = fixture(false).source("QualityChoices");
    assert_contains(
        method(source, "widenedDefault"),
        "int result = input; if (first && second) { result = 150 * input / 100; }",
    );
    assert_contains(
        method(source, "literalDefault"),
        "String result = \"/items\"; if (!first && second)",
    );
}

#[test]
fn materialized_predicates_normalize_before_control_flow_structuring() {
    for stripped in [false, true] {
        let body = method(
            fixture(stripped).source("QualityChoices"),
            "materializedPredicates",
        );
        assert_absent(body, "boolean ");
        assert_absent(body, "label");
        assert_contains(body, "< 3");
        assert_contains(body, "== 5");
    }
}

#[test]
fn crc_updates_are_single_next_value_expressions() {
    for stripped in [false, true] {
        let body = method(fixture(stripped).source("QualityChoices"), "crcUpdate");
        assert_contains(body, "for (");
        assert_absent(body, "if (");
        let update = body.lines().find(|line| line.contains(" ? ")).unwrap();
        let local = update.split('=').next().unwrap().trim();
        assert_contains(
            update,
            &format!(
                "{local} = ({local} & 1) == 1 ? ({local} >>> 1) ^ 0xedb88320 : {local} >>> 1;"
            ),
        );
    }
}

#[test]
fn large_return_decisions_keep_the_entire_guard_sequence() {
    for stripped in [false, true] {
        let body = method(fixture(stripped).source("QualityChoices"), "contentType");
        assert_eq!(body.matches("if (").count(), 6, "{body}");
        assert_absent(body, "?");
        for (suffix, result) in [
            (".txt", "text/plain"),
            (".html", "text/html"),
            (".png", "image/png"),
            (".jpg", "image/jpeg"),
            (".aac", "audio/aac"),
        ] {
            assert_contains(
                body,
                &format!(".endsWith(\"{suffix}\")) {{ return \"{result}\"; }}"),
            );
        }
    }
}

#[test]
fn independent_mask_updates_and_post_update_predicates_stay_compact() {
    for stripped in [false, true] {
        let source = fixture(stripped).source("QualityChoices");
        let mask = method(source, "maskAdjust");
        assert_contains(mask, "?");
        assert_absent(mask, "if (");
        let scan = method(source, "maskScan");
        assert_contains(scan, "do {");
        assert_absent(scan, "while (true)");
        assert_absent(scan, "continue;");
        let next = method(source, "nextOne");
        assert_contains(next, "&&");
        assert_contains(next, "++");
        assert_absent(next, "if (");
        let boxed = method(source, "boxedToggle");
        assert_contains(boxed, "new Boolean(");
        assert_absent(boxed, "$sf$");
    }
}
