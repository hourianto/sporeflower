//! Expression regions preserve ordering while recovering complete source expressions.
use crate::common::source::method;
use crate::common::{checked, sources as java_sources};
use std::{fs, path::Path, process::Command};

#[test]
fn expression_regions_preserve_effect_order_and_legacy_conversions() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    for path in [&original, &rebuilt] {
        fs::create_dir_all(path).unwrap();
    }
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&original)
            .arg("tests/fixtures/java/ExpressionRegions.java"),
    );
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("ExpressionRegions"),
    )
    .stdout;
    let api = work.path().join("api.jar");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let text = fs::read_to_string(source.join("ExpressionRegions.java")).unwrap();
    for name in [
        "summary",
        "staged",
        "primitiveSeed",
        "stringSeed",
        "charsSeed",
        "selectedChain",
        "selectedObject",
    ] {
        assert!(
            !method(&text, name).contains("StringBuffer"),
            "{name}: {text}"
        );
    }
    assert!(
        method(&text, "summary").contains("\n            +"),
        "{text}"
    );
    assert!(
        method(&text, "summary")
            .lines()
            .all(|line| line.len() < 120),
        "{text}"
    );
    assert!(
        method(&text, "objectSeed").contains("new StringBuffer"),
        "{text}"
    );
    assert!(
        method(&text, "nullableSeed").contains("new StringBuffer"),
        "{text}"
    );
    let captures = method(&text, "capturedChains");
    assert!(!captures.contains(" = "), "{captures}");
    assert_eq!(captures.matches("StringBuffer").count(), 1, "{captures}");
    let shared = method(&text, "sharedCapture");
    assert_eq!(shared.matches("item(8)").count(), 1, "{shared}");
    assert!(shared.contains(" = "), "{shared}");
    assert!(method(&text, "conjunction").contains(" && "), "{text}");
    assert!(!method(&text, "conjunction").contains("!(!"), "{text}");
    assert!(method(&text, "disjunction").contains(" || "), "{text}");
    assert_eq!(method(&text, "indexed").matches("++]").count(), 2, "{text}");
    assert_eq!(
        method(&text, "indexedDescending").matches("--]").count(),
        2,
        "{text}"
    );
    assert_eq!(
        method(&text, "indexedLoop").matches("++]").count(),
        4,
        "{text}"
    );
    assert_eq!(
        method(&text, "indexedWrites").matches("++]").count(),
        4,
        "{text}"
    );
    assert!(!method(&text, "indexedCaught").contains("++]"), "{text}");
    assert!(method(&text, "found").contains("boolean"), "{text}");
    assert!(!method(&text, "found").contains(" != 0"), "{text}");
    assert!(
        !method(&text, "numericFlag").contains("boolean v"),
        "{text}"
    );
    assert!(
        !method(&text, "noncanonical").contains("boolean v"),
        "{text}"
    );
    assert_eq!(
        method(&text, "repeated").matches("(int[])").count(),
        1,
        "{text}"
    );
    assert!(
        method(&text, "crossed").contains("(Second) (First)"),
        "{text}"
    );
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .args(java_sources(&source)),
    );
    assert_eq!(
        expected,
        checked(
            Command::new(crate::common::java())
                .arg("-cp")
                .arg(&rebuilt)
                .arg("ExpressionRegions")
        )
        .stdout
    );
    if let Some(legacy) = crate::common::legacy_javac() {
        let output = work.path().join("legacy");
        fs::create_dir_all(&output).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&output)
                .args(java_sources(&source)),
        );
        assert_eq!(
            expected,
            checked(
                Command::new(crate::common::java())
                    .arg("-cp")
                    .arg(&output)
                    .arg("ExpressionRegions")
            )
            .stdout
        );
    }
}

#[test]
fn predicate_polarity_retains_emitted_precedence_and_short_circuit_order() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let api = work.path().join("api.jar");
    fs::create_dir_all(&original).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-g:none", "-d"])
            .arg(&original)
            .args([
                "tests/fixtures/java/PredicatePrecedence.java",
                "tests/fixtures/java/PredicatePrecedenceDriver.java",
            ]),
    );
    let run = |classes: &Path| {
        checked(
            Command::new(crate::common::java())
                .args(["-Xverify:all", "-cp"])
                .arg(classes)
                .arg("PredicatePrecedenceDriver"),
        )
        .stdout
    };
    let expected = String::from_utf8(run(&original)).unwrap();
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(original.join("PredicatePrecedence.class"))
            .arg(&source)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let emitted = fs::read_to_string(source.join("PredicatePrecedence.java")).unwrap();
    for legacy in [false, true] {
        let rebuilt = work.path().join(if legacy { "legacy" } else { "modern" });
        fs::create_dir_all(&rebuilt).unwrap();
        if legacy {
            let Some(compiler) = crate::common::legacy_javac() else {
                continue;
            };
            checked(
                crate::common::legacy_command(compiler, &api)
                    .arg("-d")
                    .arg(&rebuilt)
                    .arg(source.join("PredicatePrecedence.java")),
            );
        } else {
            checked(
                crate::common::compile_command()
                    .args(["-d"])
                    .arg(&rebuilt)
                    .arg(source.join("PredicatePrecedence.java")),
            );
        }
        fs::copy(
            original.join("PredicatePrecedenceDriver.class"),
            rebuilt.join("PredicatePrecedenceDriver.class"),
        )
        .unwrap();
        let actual = String::from_utf8(run(&rebuilt)).unwrap();
        assert_eq!(expected.lines().count(), actual.lines().count());
        for (expected, actual) in expected.lines().zip(actual.lines()) {
            assert_eq!(expected, actual, "legacy={legacy}\n{emitted}");
        }
    }
}

#[test]
fn indexed_updates_share_lifetimes_without_crossing_observable_writes() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    let api = work.path().join("api.jar");
    crate::common::compile(
        &["tests/fixtures/java/IndexOwnership.java".into()],
        &original,
        None,
    );
    let expected = crate::common::execute(&original, "IndexOwnership");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .arg("--stubs")
            .arg(&api),
    );
    let text = fs::read_to_string(source.join("IndexOwnership.java")).unwrap();
    for name in ["protectedBranches", "readChunks"] {
        assert!(method(&text, name).contains("++]"), "{name}: {text}");
    }
    assert!(!method(&text, "observedDelay").contains("++]"), "{text}");
    assert!(method(&text, "readChunks").contains("do {"), "{text}");
    crate::common::compile(&java_sources(&source), &rebuilt, None);
    assert_eq!(expected, crate::common::execute(&rebuilt, "IndexOwnership"));
    if let Some(legacy) = crate::common::legacy_javac() {
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&rebuilt)
                .args(java_sources(&source)),
        );
        assert_eq!(expected, crate::common::execute(&rebuilt, "IndexOwnership"));
    }
}
