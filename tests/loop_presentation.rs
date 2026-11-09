//! Loop spelling preserves the exact continue boundary, side effects and the
//! lexical scope of a terminal test under modern and Java 1.3 compilers.
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
    checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(output)
            .arg("LoopPresentation"),
    )
}

#[test]
fn loop_forms_preserve_actions_updates_scope_and_continue_ownership() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("LoopPresentation.java");
    fs::write(&input, include_str!("fixtures/LoopPresentation.java")).unwrap();
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
    let source = generated.join("LoopPresentation.java");
    let text = fs::read_to_string(&source).unwrap();
    for name in [
        "count",
        "flags",
        "filteredEffects",
        "largerGuard",
        "compoundGuard",
        "matchingTail",
        "alternativeTail",
        "floatingTail",
    ] {
        let body = method(&text, name);
        assert!(!body.contains("continue;"), "{name}: {body}");
        assert!(body.contains("if ("), "{name}: {body}");
    }
    assert!(method(&text, "count").contains("!= null"), "{text}");
    assert!(method(&text, "alternativeTail").contains("else"), "{text}");
    let large = method(&text, "largeTail");
    assert!(large.contains("continue;"), "{large}");
    for name in [
        "postTest",
        "nestedPostTest",
        "lazyPostTest",
        "protectedPostTest",
        "readAfterPostTest",
        "scopedTest",
        "retainedScopedTest",
    ] {
        let body = method(&text, name);
        assert!(body.contains("do {"), "{name}: {body}");
        assert!(body.contains("} while ("), "{name}: {body}");
    }
    let scoped = method(&text, "scopedTest");
    assert!(
        scoped.contains("} while (LoopPresentation.effect(i) < limit);"),
        "{scoped}"
    );
    assert_eq!(scoped.matches("LoopPresentation.effect(").count(), 1);
    let protected = method(&text, "protectedPostTest");
    assert!(
        protected
            .replace("LoopPresentation.", "")
            .contains("synchronized (lock)"),
        "each iteration acquires the field directly without an unused capture: {protected}"
    );
    for name in ["skipTest", "labelledSkip"] {
        let body = method(&text, name);
        assert!(body.contains("while (true)"), "{name}: {body}");
        assert!(!body.contains("do {"), "{name}: {body}");
    }
    for name in ["countdown", "protectedCountdown", "wrappingCountdown"] {
        let body = method(&text, name);
        assert!(body.contains("while (--limit >= 0)"), "{name}: {body}");
    }
    for name in ["boundedIndex", "boundedFlag", "boundedCounterReturn"] {
        let body = method(&text, name);
        assert!(body.contains("index < values.length"), "{name}: {body}");
        assert!(
            !body.contains("; true;") && !body.contains("break;"),
            "{name}: {body}"
        );
        assert!(!body.contains("found"), "{name}: {body}");
    }
    for name in ["effectfulSearch", "repeatedSearch"] {
        let body = method(&text, name);
        assert!(body.contains("< values.length"), "{name}: {body}");
        assert!(!body.contains("true;"), "{name}: {body}");
        assert!(!body.contains("while (true)"), "{name}: {body}");
    }
    assert_eq!(
        method(&text, "persistentMatch").matches("boolean ").count(),
        1,
        "{text}"
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
