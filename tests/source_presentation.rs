//! Final source choices retain short-circuit order, assignment conversions and
//! the one monitor evaluation, including the Java 1.3 conditional type rules.
use crate::common::checked_stdout as checked;
use crate::common::source::method;
use std::{fs, path::Path, process::Command};

fn compile(source: &Path, output: &Path) {
    fs::create_dir_all(output).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-g", "-d"])
            .arg(output)
            .arg(source),
    );
}

fn run(output: &Path) -> Vec<u8> {
    crate::common::execute(output, "SourcePresentation")
}

#[test]
fn source_choices_preserve_lazy_effects_types_and_monitor_identity() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("SourcePresentation.java");
    let fixture = r#"
public class SourcePresentation {
    static int trace;
    static final Object first = new Object();
    static final Object second = new Object();
    static Object field = first;
    static boolean step(int value) { trace = trace * 7 + value; return (value & 1) != 0; }
    static Object lock() { trace = trace * 7 + 3; return field; }
    static boolean pair(int kind, int left, int right) {
        if (kind != 1) { if (left != 3) return false; return true; }
        if (right != 3) return false;
        return true;
    }
    static boolean either(int value) {
        if (!step(value)) return false;
        if (value == 5 || step(value + 2)) return true;
        return false;
    }
    static boolean equalResults(int value) {
        if (step(value)) return true;
        return true;
    }
    static byte narrow(boolean choose) { if (choose) return 1; return 2; }
    static double mixed(boolean choose) { if (choose) return 16777217; return 1.5F; }
    static Object references(boolean choose) { if (choose) return "yes"; return new StringBuffer("no"); }
    static int many(int a, int b, int c, int d, int e, int f, int g, int h) { return a + b + c + d + e + f + g + h; }
    static int wide(boolean choose) {
        if (choose) return many(effect(100001), effect(100002), effect(100003), effect(100004), effect(100005), effect(100006), effect(100007), effect(100008));
        return many(effect(200001), effect(200002), effect(200003), effect(200004), effect(200005), effect(200006), effect(200007), effect(200008));
    }
    static int longGuards(int value) {
        // LONG_GUARDS
        return effect(100);
    }
    static int selected(boolean choose) { if (choose) return effect(3); else return effect(5); }
    static int effect(int value) { trace = trace * 7 + value; return value; }
    static int guarded(boolean choose) {
        try { if (choose) return 12 / trace; else return effect(9); }
        catch (ArithmeticException e) { return effect(11); }
    }
    static int monitor() {
        Object saved = lock();
        synchronized (saved) { field = second; return trace; }
    }
    static boolean retainedMonitor() {
        Object saved = lock();
        synchronized (saved) { field = second; }
        return saved == first;
    }
    static int counts(int limit) {
        int total = 0;
        for (int i = 0; i < limit; ++i) { if (i % 3 == 0) continue; total += effect(i); }
        return total;
    }
    static String groupedCases(int value) {
        String result;
        switch (value) {
            case 0: result = "Undefined"; break;
            case 1: result = "one"; break;
            case 2: result = "Undefined"; break;
            case 3: result = "Undefined"; break;
            default: result = "Undefined"; break;
        }
        effect(value);
        return result;
    }
    static int enteringCase(int value) {
        int result = 0;
        switch (value) {
            case 0: effect(2);
            case 1: result = 7; break;
            case 2: result = 7; break;
            case 3: result = 9;
            default: break;
        }
        return result;
    }
    static int abruptCases(int value) {
        switch (value) {
            case 0: if (trace == 0) throw new IllegalArgumentException(); else return effect(2);
            default: throw new IllegalArgumentException();
            case 1: break;
        }
        return effect(3);
    }
    static int combined(int value) {
        try {
            try { effect(12 / value); }
            catch (ArithmeticException e) { effect(9); }
        } finally { effect(13); }
        return trace;
    }
    static Object adjacentResult(int value) {
        try {
            Object result = new StringBuffer().append(effect(12 / value));
            return result;
        } catch (ArithmeticException e) { return "zero"; }
    }
    static int repeatedFacts(int value) {
        if (value == -1024) return 0;
        effect(1);
        if (value == -1024) return 1;
        if (value == -1024) return 2;
        if (value == -1024) return 3;
        return value;
    }
    static int changedFacts(int value, boolean change) {
        if (value == 2) return effect(2);
        if (change) value = 2;
        if (value == 2) return effect(3);
        for (int i = 0; i < 2; i++) {
            if (value == 2) effect(4);
            value = 2;
        }
        return effect(value);
    }
    static void checkedCall() throws java.io.IOException { throw new java.io.IOException(); }
    static int prunedChecked(int value) {
        try {
            if (value == 2) return 7;
            effect(1);
            if (value == 2) checkedCall();
            return 9;
        } catch (java.io.IOException e) { return 11; }
    }
    static int protectedFacts(int value) {
        if (value == 2) return 1;
        try {
            value = 2;
            effect(10 / trace);
        } catch (ArithmeticException e) {
            if (value == 2) return 2;
        } finally { effect(3); }
        return value;
    }
    static int expressionWrite(int value) {
        if (value == 2) return 1;
        effect(value = 2);
        if (value == 2) return 2;
        return 3;
    }
    static int compoundFacts(int value) {
        if (value == 1 && (value = 2) > 0) {
            if (value == 1) return 1;
            return value;
        }
        return value;
    }
    public static void main(String[] args) {
        for (int i = 0; i < 7; ++i) {
            trace = 0;
            System.out.println(pair(i, i, 3) + ":" + either(i) + ":" + equalResults(i) + ":" + trace);
            System.out.println(narrow((i & 1) == 0) + ":" + mixed((i & 1) == 0) + ":" + references((i & 1) == 0));
            System.out.println(selected((i & 1) == 0) + ":" + guarded((i & 1) == 0) + ":" + counts(i) + ":" + trace);
            System.out.println(wide((i & 1) == 0) + ":" + trace);
            System.out.println(groupedCases(i) + ":" + enteringCase(i) + ":" + combined(i) + ":" + adjacentResult(i) + ":" + trace);
            System.out.println(repeatedFacts(i) + ":" + changedFacts(i, (i & 1) == 0) + ":" + expressionWrite(i) + ":" + compoundFacts(i) + ":" + trace);
            trace = 0;
            System.out.println(protectedFacts(i) + ":" + prunedChecked(i) + ":" + trace);
            try { System.out.println(abruptCases(i) + ":" + trace); }
            catch (IllegalArgumentException e) { System.out.println("abrupt:" + trace); }
            field = first;
            System.out.println(monitor() + ":" + trace);
            field = first;
            System.out.println(retainedMonitor() + ":" + trace);
        }
        field = null;
        try { monitor(); } catch (NullPointerException e) { System.out.println("null:" + trace); }
        trace = 0;
        System.out.println(guarded(true) + ":" + trace);
        for (int i = 0; i <= 100; ++i) System.out.println(longGuards(i) + ":" + trace);
    }
}

"#;
    let guards = (0..100)
        .map(|value| format!("if (value == {value}) return effect({value});"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&input, fixture.replace("// LONG_GUARDS", &guards)).unwrap();
    let original = work.path().join("original");
    compile(&input, &original);
    let expected = run(&original);
    let generated = work.path().join("source");
    let api = work.path().join("api.jar");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&generated)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let source = generated.join("SourcePresentation.java");
    let text = fs::read_to_string(&source).unwrap();
    let pair = method(&text, "pair");
    assert!(!pair.contains("if ("), "{pair}");
    assert!(pair.contains(" ? "), "{pair}");
    let either = method(&text, "either");
    assert!(!either.contains("if ("), "{either}");
    assert!(either.contains("&&") && either.contains("||"), "{either}");
    let equal = method(&text, "equalResults");
    assert!(
        equal.contains("step("),
        "condition effects disappeared: {equal}"
    );
    let monitor = method(&text, "monitor");
    assert!(
        monitor.contains("synchronized (SourcePresentation.lock())"),
        "{monitor}"
    );
    assert!(!monitor.contains("Object saved"), "{monitor}");
    let retained = method(&text, "retainedMonitor");
    assert!(retained.contains("synchronized (saved)"), "{retained}");
    let long = method(&text, "longGuards");
    assert!(
        long.contains("if ("),
        "large guard sequences must stay bounded: {long}"
    );
    let wide = method(&text, "wide");
    assert!(
        wide.contains("if (choose)") && !wide.contains('?'),
        "{wide}"
    );
    assert_eq!(
        wide.matches("return SourcePresentation.many(").count(),
        2,
        "{wide}"
    );
    assert!(wide.lines().all(|line| line.len() < 150), "{wide}");
    let grouped = method(&text, "groupedCases");
    assert_eq!(grouped.matches("\"Undefined\"").count(), 1, "{grouped}");
    let combined = method(&text, "combined");
    assert_eq!(combined.matches("try {").count(), 1, "{combined}");
    assert!(
        combined.contains("catch (") && combined.contains("finally {"),
        "{combined}"
    );
    let adjacent = method(&text, "adjacentResult");
    assert!(!adjacent.contains("return result;"), "{adjacent}");
    let repeated = method(&text, "repeatedFacts");
    assert_eq!(repeated.matches("-1024").count(), 1, "{repeated}");
    let pruned = method(&text, "prunedChecked");
    assert!(
        !pruned.contains("checkedCall("),
        "path-fact pruning failed: {pruned}"
    );
    assert!(
        pruned.contains("if (false)") && pruned.contains("catch ("),
        "the selected source body must witness its retained checked catch: {pruned}"
    );
    let rebuilt = work.path().join("rebuilt");
    compile(&source, &rebuilt);
    assert_eq!(expected, run(&rebuilt));
    if let Some(legacy) = crate::common::legacy_javac() {
        let rebuilt = work.path().join("legacy");
        fs::create_dir_all(&rebuilt).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&rebuilt)
                .arg(&source),
        );
        assert_eq!(expected, run(&rebuilt));
    }
}

#[test]
fn void_cleanup_composition_preserves_completion_and_handler_boundaries() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("VoidCompletion.java");
    fs::write(
        &input,
        r#"
public class VoidCompletion {
    static int trace;
    static final Object lock = new Object();
    static void effect(int value) { trace = trace * 7 + value; }
    static void work(boolean fail) throws java.io.IOException {
        effect(1);
        if (fail) throw new java.io.IOException();
    }
    static boolean test(boolean fail) { effect(6); return fail; }
    static void bareGuard(boolean fail) { if (test(fail)) return; }
    static void tail(boolean fail) {
        try {
            try { work(fail); }
            catch (java.io.IOException e) { effect(2); return; }
            return;
        } finally { effect(3); }
    }
    static void guarded(boolean enter, boolean fail) {
        if (enter) {
            try {
                try { work(fail); }
                catch (java.io.IOException e) { effect(2); return; }
                return;
            } finally { effect(3); }
        }
    }
    static void monitored(boolean enter, boolean fail) {
        if (enter) {
            synchronized (lock) {
                try {
                    try { work(fail); }
                    catch (java.io.IOException e) { effect(2); return; }
                    return;
                } finally { effect(3); }
            }
        }
    }
    static void early(boolean enter, boolean fail) {
        if (enter) {
            try {
                try { work(fail); }
                catch (java.io.IOException e) { effect(2); return; }
                return;
            } finally { effect(3); }
        }
        effect(4);
    }
    static void loop(boolean fail) {
        for (int i = 0; i < 2; i++) {
            try { work(fail); }
            catch (java.io.IOException e) { effect(2); return; }
            finally { effect(3); }
        }
        effect(5);
    }
    static void cases(int value, boolean fail) {
        switch (value) {
            case 0:
                try { work(fail); }
                catch (java.io.IOException e) { effect(2); return; }
                finally { effect(3); }
            case 1: effect(4);
        }
        effect(5);
    }
    static void override(boolean fail) {
        try {
            try { work(fail); }
            catch (java.io.IOException e) { throw new IllegalStateException(); }
            return;
        } finally { effect(3); return; }
    }
    static void throwing(boolean fail) throws java.io.IOException {
        try {
            try { work(fail); }
            catch (java.io.IOException e) { effect(2); return; }
            work(true);
        } finally { effect(3); }
    }
    static int saved(boolean fail) {
        int value = 1;
        try {
            try { work(fail); }
            catch (java.io.IOException e) { value = 2; }
            return value;
        } finally { value = 3; effect(value); }
    }
    public static void main(String[] args) {
        for (int i = 0; i < 4; i++) {
            boolean enter = (i & 1) != 0;
            boolean fail = (i & 2) != 0;
            trace = 0; bareGuard(fail); System.out.println(trace);
            trace = 0; tail(fail); System.out.println(trace);
            trace = 0; guarded(enter, fail); System.out.println(trace);
            trace = 0; monitored(enter, fail); System.out.println(trace);
            trace = 0; early(enter, fail); System.out.println(trace);
            trace = 0; loop(fail); System.out.println(trace);
            trace = 0; cases(i % 3, fail); System.out.println(trace);
            trace = 0; override(fail); System.out.println(trace);
            trace = 0;
            try { throwing(fail); } catch (java.io.IOException e) { effect(6); }
            System.out.println(trace);
            trace = 0; System.out.println(saved(fail) + ":" + trace);
        }
    }
}
"#,
    )
    .unwrap();
    let original = work.path().join("original");
    compile(&input, &original);
    let expected = crate::common::execute(&original, "VoidCompletion");
    let generated = work.path().join("source");
    let api = work.path().join("api.jar");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&generated)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let source = generated.join("VoidCompletion.java");
    let text = fs::read_to_string(&source).unwrap();
    for name in ["tail", "guarded", "early"] {
        let body = method(&text, name);
        assert_eq!(body.matches("try {").count(), 1, "{name}: {body}");
        assert!(
            body.contains("catch (") && body.contains("finally {"),
            "{body}"
        );
        if name != "early" {
            assert!(!body.contains("return;"), "{name}: {body}");
        }
    }
    for name in ["bareGuard", "early", "loop", "cases"] {
        let body = method(&text, name);
        assert!(body.contains("return;"), "{name}: {body}");
    }
    let throwing = method(&text, "throwing");
    assert_eq!(throwing.matches("try {").count(), 2, "{throwing}");
    let rebuilt = work.path().join("rebuilt");
    compile(&source, &rebuilt);
    assert_eq!(expected, crate::common::execute(&rebuilt, "VoidCompletion"));
    if let Some(legacy) = crate::common::legacy_javac() {
        let rebuilt = work.path().join("legacy");
        fs::create_dir_all(&rebuilt).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&rebuilt)
                .arg(&source),
        );
        assert_eq!(expected, crate::common::execute(&rebuilt, "VoidCompletion"));
    }
}
