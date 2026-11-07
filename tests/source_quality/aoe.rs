//! Reduced AoE3-style dispatch and objective updates: unchanged switch arms
//! retain existing locals, while fresh dispatch and search results own defaults.
use super::support::{
    Fixture, assert_absent, assert_contains as assert_syntax, assert_no_word, compact, method,
    word_count,
};
use std::sync::OnceLock;

fn fixture(stripped: bool) -> &'static Fixture {
    static NAMED: OnceLock<Fixture> = OnceLock::new();
    static STRIPPED: OnceLock<Fixture> = OnceLock::new();
    if stripped {
        STRIPPED.get_or_init(|| {
            Fixture::build_stripped("tests/fixtures/source_quality/aoe", "QualityAoe")
        })
    } else {
        NAMED.get_or_init(|| Fixture::build("tests/fixtures/source_quality/aoe", "QualityAoe"))
    }
}

fn body(name: &str) -> &'static str {
    method(fixture(false).source("QualityAoe"), name)
}

fn assert_contains(body: &str, snippet: &str) {
    assert_syntax(&body.replace("QualityAoe.", ""), snippet);
}

#[test]
fn menu_dispatch_updates_its_action_parameter_in_place() {
    let body = body("menuAction");
    assert_contains(body, "switch (action)");
    for value in [30, 17, 57] {
        assert_contains(body, &format!("action = {value};"));
    }
    assert_contains(body, "observe(action); return action;");
    assert_no_word(body, "int");
    assert_absent(body, "= action;");
    assert_absent(body, "?");
}

#[test]
fn unchanged_menu_action_keeps_one_guarded_effect() {
    let body = body("menuAction");
    assert_contains(body, "else if (frame + 1 == frameCount) { observe(3); }");
    assert_eq!(compact(body).matches("observe(3)").count(), 1, "{body}");
}

#[test]
fn objective_switch_updates_exactly_the_two_existing_flags() {
    let body = body("objectiveFlags");
    assert_contains(body, "boolean complete = objective[1] == 1;");
    assert_contains(body, "boolean failed = objective[1] == 2;");
    assert_contains(body, "complete = true;");
    assert_contains(body, "failed = true;");
    assert_contains(body, "if (complete) { objective[1] = 1;");
    assert_contains(body, "if (failed) { objective[1] = 2;");
    assert_eq!(word_count(body, "boolean"), 2, "{body}");
    assert_absent(body, "= complete;");
    assert_absent(body, "= failed;");
    assert_no_word(body, "else");
}

#[test]
fn carried_loop_flag_keeps_the_bounded_search_and_parameter() {
    let body = body("retainedFlag");
    assert_contains(body, "for (int index = 0; index < states.length; index++)");
    assert_contains(body, "failed = true;");
    assert_contains(body, "return failed;");
    assert_no_word(body, "boolean");
    assert_absent(body, "; true;");
}

#[test]
fn fresh_dispatch_result_still_has_one_owned_default() {
    let body = body("freshResult");
    let (initializers, _) = body.split_once("switch").unwrap();
    assert_contains(initializers, "int result =");
    assert_contains(body, "observe(result); return result;");
    assert_eq!(word_count(body, "int"), 1, "{body}");
}

#[test]
fn copying_an_independent_input_does_not_turn_a_fresh_result_into_that_parameter() {
    let body = body("freshCopiedResult");
    let (initializers, _) = body.split_once("switch").unwrap();
    assert_contains(initializers, "int result =");
    assert_contains(body, "result = fallback;");
    assert_contains(body, "observe(result); return result;");
    assert_absent(body, "fallback =");
    assert_eq!(word_count(body, "int"), 1, "{body}");
}

#[test]
fn new_search_result_stays_separate_from_its_bounded_induction() {
    let body = body("searchResult");
    assert_contains(body, "= -1;");
    assert_contains(body, "< values.length;");
    assert_contains(body, "++");
    assert_absent(body, "; true;");
    assert_absent(body, "while (true)");
    assert_absent(body, ": {");
}

#[test]
fn stripped_bytecode_keeps_dispatch_storage_without_name_hints() {
    let source = fixture(true).source("QualityAoe");
    let action = method(source, "menuAction");
    assert_contains(action, "switch (");
    assert_no_word(action, "int");
    assert_absent(action, "?");
    let flags = method(source, "objectiveFlags");
    assert_eq!(word_count(flags, "boolean"), 2, "{flags}");
    assert_no_word(flags, "else");
    let copied = method(source, "freshCopiedResult");
    let (initializers, _) = copied.split_once("switch").unwrap();
    assert_eq!(word_count(initializers, "int"), 1, "{initializers}");
    assert_eq!(word_count(copied, "int"), 1, "{copied}");
}

// Legacy inlining chooses true or the existing flag on the operand stack;
// javac instead stores the loop's result. The switch and the scan share one
// source flag even though the final stack join has no JVM local slot.
fn stack_scan_class() -> Vec<u8> {
    let code = [
        0x03, 0x36, 0x04, // 0: failed = false
        0x1a, 0xaa, 0, 0, 0, // 3: switch (mode), aligned tableswitch
        0, 0, 0, 0x35, // default -> 57
        0, 0, 0, 0, 0, 0, 0, 1, // cases 0..1
        0, 0, 0, 0x18, 0, 0, 0, 0x2a, // cases -> 28, 46
        0x1b, 0x9d, 0, 0x0b, // 28: buildings > 0 -> false
        0x1c, 0x9d, 0, 7, // 32: units > 0 -> false
        0x04, 0xa7, 0, 4, 0x03, // 36: choose true/false
        0x36, 4, 0xa7, 0, 0x0e, // 41: failed = choice; goto 57
        0x1b, 0x9d, 0, 7, // 46: buildings > 0 -> false
        0x04, 0xa7, 0, 4, 0x03, 0x36, 4, // 50: failed = choice
        0x03, 0x36, 5, // 57: index = 0
        0x15, 5, 0x2d, 0xbe, 0xa2, 0, 0x15, // 60: index >= states.length -> 85
        0x2d, 0x15, 5, 0x2e, 0x05, 0xa0, 0, 7, // 67: states[index] != 2 -> 79
        0x04, 0xa7, 0, 0x0b, // 75: true stays on stack; goto consumer
        0x84, 5, 1, 0xa7, 0xff, 0xea, // 79: index++; goto loop
        0x15, 4, // 85: previous failed value stays on stack
        0x99, 0, 6, 0x10, 7, 0xac, 0x06, 0xac, // 87: return choice ? 7 : 3
    ];
    let mut bytes = vec![0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 49, 0, 8];
    for (index, value) in [
        (1, "StackScan"),
        (3, "java/lang/Object"),
        (5, "scan"),
        (6, "(III[I)I"),
        (7, "Code"),
    ] {
        bytes.push(1);
        bytes.extend((value.len() as u16).to_be_bytes());
        bytes.extend(value.as_bytes());
        if index < 5 {
            bytes.extend([7, 0, index]);
        }
    }
    bytes.extend([0, 0x31, 0, 2, 0, 4, 0, 0, 0, 0, 0, 1]);
    bytes.extend([0, 9, 0, 5, 0, 6, 0, 1, 0, 7]); // public static scan; Code
    bytes.extend(((code.len() + 12) as u32).to_be_bytes());
    bytes.extend([0, 3, 0, 6]); // max stack, max locals
    bytes.extend((code.len() as u32).to_be_bytes());
    bytes.extend(code);
    bytes.extend([0, 0, 0, 0, 0, 0]); // no handlers or nested/class attributes
    bytes
}

#[test]
fn switch_flag_flows_into_an_inlined_scan_without_new_storage() {
    use std::fs;
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    fs::create_dir_all(&original).unwrap();
    fs::write(original.join("StackScan.class"), stack_scan_class()).unwrap();
    let driver = work.path().join("StackScanDriver.java");
    fs::write(
        &driver,
        r#"
        public class StackScanDriver {
            public static void main(String[] args) {
                int[][] scans = {{}, {0}, {2}, {0, 2}, {2, 0}};
                for (int mode = -1; mode < 3; mode++) {
                    for (int buildings = 0; buildings < 2; buildings++) {
                        for (int units = 0; units < 2; units++) {
                            for (int row = 0; row < scans.length; row++) {
                                boolean failed = mode == 0 ? buildings <= 0 && units <= 0
                                    : mode == 1 && buildings <= 0;
                                for (int index = 0; index < scans[row].length; index++) {
                                    if (scans[row][index] == 2) failed = true;
                                }
                                int result = StackScan.scan(mode, buildings, units, scans[row]);
                                if (result != (failed ? 7 : 3)) throw new RuntimeException();
                                System.out.println(result);
                            }
                        }
                    }
                }
            }
        }
    "#,
    )
    .unwrap();
    crate::common::compile(&[driver], &original, None);
    let fixture = Fixture::from_classes(&original, "StackScanDriver");
    let body = method(fixture.source("StackScan"), "scan");
    assert_contains(body, "switch (");
    assert_contains(body, "for (");
    assert_contains(body, ".length;");
    assert_eq!(word_count(body, "boolean"), 1, "{body}");
    assert_absent(body, "; true;");
    assert_absent(body, "while (true)");
}
