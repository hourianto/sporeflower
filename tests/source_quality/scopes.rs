//! Readable cleanup, lexical lifetimes, allocations and member bindings.
use super::support::{Fixture, assert_absent, assert_contains, assert_no_word, compact, method};
use std::sync::OnceLock;

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| Fixture::build("tests/fixtures/source_quality/scopes", "ScopeQuality"))
}

fn body(name: &str) -> &'static str {
    static SOURCE: OnceLock<String> = OnceLock::new();
    // These unshadowed helper calls are equally clear with or without their
    // owner. Keep field qualification intact for the binding tests below.
    let source = SOURCE.get_or_init(|| {
        fixture()
            .source("ScopeQuality")
            .replace("ScopeQuality.checkedValue(", "checkedValue(")
            .replace("ScopeQuality.mark(", "mark(")
            .replace("ScopeQuality.acquire(", "acquire(")
    });
    method(source, name)
}

#[test]
fn return_expression_stays_inside_one_finally() {
    let body = body("returnFinally");
    // javac saves the return value and copies cleanup onto normal and throw
    // exits; none of that machinery is needed in the reconstructed source.
    assert_contains(body, "try { return checkedValue(value);");
    assert_contains(body, "finally { mark(9);");
    assert_eq!(compact(body).matches("try{").count(), 1, "{body}");
    assert_eq!(compact(body).matches("finally{").count(), 1, "{body}");
    assert_eq!(body.matches("mark(9)").count(), 1, "{body}");
    assert_absent(body, "catch (");
    assert!(
        !body.lines().any(|line| line.trim().starts_with("int ")),
        "{body}"
    );
}

#[test]
fn caught_work_and_cleanup_share_one_try() {
    let body = body("combinedCleanup");
    assert_contains(body, "try { checkedValue(value);");
    assert_contains(body, "catch (IOException problem) {");
    assert_contains(body, "mark(problem.getMessage().length());");
    assert_contains(body, "finally { mark(8);");
    assert_eq!(compact(body).matches("try{").count(), 1, "{body}");
    assert_eq!(body.matches("mark(8)").count(), 1, "{body}");
    assert_absent(body, "catch (Throwable");
}

#[test]
fn checked_rethrow_uses_the_named_catch_binding() {
    let body = body("rethrowChecked");
    assert_contains(body, "catch (IOException problem) {");
    assert_contains(body, "mark(7); throw problem;");
    assert_absent(body, "throw (IOException)");
    assert_absent(body, "new IOException");
    assert_absent(body, "$caught");
}

#[test]
fn side_effecting_monitor_expression_is_used_once_without_an_alias() {
    let body = body("monitorExpression");
    // The monitorenter/monitorexit alias and catch-all are compiler artifacts.
    assert_contains(body, "synchronized (acquire()) {");
    assert_contains(body, "mark(value);");
    assert_eq!(body.matches("acquire()").count(), 1, "{body}");
    assert_no_word(body, "Object");
    assert_absent(body, "try {");
    assert_absent(body, "catch (");
}

#[test]
fn synchronized_cleanup_keeps_the_handler_inside_the_monitor() {
    let body = body("monitorCleanup");
    assert_contains(body, "synchronized (lock) { try {");
    assert_contains(body, "catch (IOException problem) {");
    assert_contains(body, "finally { mark(6);");
    assert_eq!(compact(body).matches("try{").count(), 1, "{body}");
    assert_eq!(body.matches("mark(6)").count(), 1, "{body}");
    assert_absent(body, "catch (Throwable");
    assert_no_word(body, "break");
}

#[test]
fn used_monitor_binding_remains_a_named_local() {
    let body = body("retainedMonitor");
    assert_contains(body, "Object saved = acquire();");
    assert_contains(body, "synchronized (saved) { saved.notifyAll();");
    assert_eq!(body.matches("acquire()").count(), 1, "{body}");
    assert_absent(body, "Object saved;");
    assert_absent(body, "catch (");
}

#[test]
fn shared_call_result_is_declared_at_its_first_assignment() {
    let body = body("declarationNearUse");
    assert_contains(body, "mark(1); int measured = mark(value);");
    assert_contains(body, "return measured + measured;");
    assert_eq!(body.matches("mark(value)").count(), 1, "{body}");
    assert_absent(body, "int measured;");
    assert_absent(body, "measured = 0;");
}

#[test]
fn branch_owned_local_is_not_hoisted_above_its_guard() {
    let body = body("branchLocal");
    assert_contains(body, "int measured = mark(value);");
    assert_contains(body, "mark(measured); return measured + 1;");
    let syntax = compact(body);
    assert!(
        syntax.find("if(").unwrap() < syntax.find("intmeasured=").unwrap(),
        "{body}"
    );
    assert_eq!(body.matches("mark(value)").count(), 1, "{body}");
    assert_absent(body, "int measured;");
    assert_absent(body, "measured = 0;");
}

#[test]
fn reused_local_slot_keeps_separate_typed_debug_names() {
    let body = body("siblingLocals");
    // Both lifetimes occupy bytecode slot 1. Their types and names should not
    // collapse into a method-wide Object storage slot and casts.
    assert_contains(body, "int width = mark(3);");
    assert_contains(body, "String title = String.valueOf(mark(4));");
    assert_contains(body, "return width * width;");
    assert_contains(body, "return title.length() + title.length();");
    assert_absent(body, "int width;");
    assert_absent(body, "String title;");
    assert_no_word(body, "Object");
    assert_absent(body, "(String)");
}

#[test]
fn handler_local_is_initialized_within_its_named_catch() {
    let body = body("catchLocal");
    assert_contains(body, "try { return checkedValue(value);");
    assert_contains(
        body,
        "catch (IOException problem) { String message = problem.getMessage();",
    );
    assert_contains(body, "return message.length() + message.length();");
    assert_eq!(body.matches("getMessage()").count(), 1, "{body}");
    assert_absent(body, "String message;");
    assert_absent(body, "message = null;");
}

#[test]
fn exception_visible_assignment_reuses_one_initialized_binding() {
    let body = body("preservedCommit");
    assert_contains(
        body,
        "int committed = 11; try { committed = checkedValue(value);",
    );
    assert_contains(body, "mark(committed);");
    assert_contains(body, "return committed;");
    assert_eq!(compact(body).matches("intcommitted").count(), 1, "{body}");
    assert_absent(body, "int committed;");
    assert_absent(body, "committed = 0;");
}

#[test]
fn ordinary_and_nested_allocations_remain_constructor_expressions() {
    let direct = body("allocatedValue");
    assert_contains(direct, "return new ScopeProduct(mark(value)).value;");
    let nested = body("nestedAllocation");
    assert_contains(
        nested,
        "new ScopeEnvelope(new ScopeProduct(mark(value)), mark(4)).read()",
    );
    for body in [direct, nested] {
        assert_eq!(body.matches("mark(value)").count(), 1, "{body}");
        assert_absent(body, "$sf$");
        assert!(
            !body
                .lines()
                .any(|line| line.trim().starts_with("ScopeProduct ")),
            "{body}"
        );
    }
}

#[test]
fn primitive_array_stores_become_one_ordered_initializer() {
    let body = body("initializedArray");
    assert_contains(body, "new int[] { mark(value), mark(2), mark(3) }");
    assert_absent(body, "new int[3]");
    assert_absent(body, "[0] =");
    assert_absent(body, "[1] =");
    assert_absent(body, "[2] =");
}

#[test]
fn object_array_initializer_keeps_its_constructor_expressions() {
    let body = body("initializedObjects");
    assert_contains(
        body,
        "new ScopeProduct[] { new ScopeProduct(mark(value)), new ScopeProduct(mark(2)) }",
    );
    assert_absent(body, "new ScopeProduct[2]");
    assert_absent(body, "[0] =");
    assert_absent(body, "$sf$");
}

#[test]
fn jagged_array_initializers_do_not_expand_into_row_stores() {
    let body = body("initializedRows");
    assert_contains(
        body,
        "new int[][] { { mark(value), mark(2) }, { mark(3) } }",
    );
    assert_absent(body, "[0] =");
    assert_absent(body, "[1] =");
    assert_absent(body, "new int[2]");
}

#[test]
fn parameter_shadowing_keeps_natural_instance_and_static_qualifiers() {
    let body = body("shadowedMembers");
    assert_contains(body, "this.value + value");
    assert_contains(body, "ScopeQuality.total + total");
    assert_absent(body, "((ScopeQuality)");
    assert_absent(body, "value2");
    assert_absent(body, "total2");
}

#[test]
fn hidden_parent_members_use_super_without_receiver_casts() {
    let body = body("inheritedBinding");
    assert_contains(body, "super.value + this.value + super.read()");
    assert_absent(body, "(ScopeParent)");
    assert_absent(body, "$sf$");
}

#[test]
fn constructor_chaining_and_final_assignment_remain_direct() {
    let source = fixture().source("ScopeQuality");
    let start = source.find(" ScopeQuality(int ").unwrap();
    let constructor = method(&source[start..], "ScopeQuality");
    assert_contains(constructor, "super(value + 1);");
    assert_contains(constructor, "this.value = value;");
    assert_contains(source, "ScopeQuality() { this(5);");
    assert_contains(source, "final int value;");
    assert_absent(constructor, "$sf$");
    assert_absent(constructor, "(ScopeParent)");
}
