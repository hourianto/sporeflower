use crate::common::checked_stdout as checked;
use std::{fs, process::Command};

#[test]
fn same_type_nested_catches_keep_one_enclosing_scope_and_exception_order() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-g:none", "-d"])
            .arg(&original)
            .arg("tests/fixtures/java/NestedCatchScopes.java"),
    );
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("NestedCatchScopes"),
    );
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(original.join("NestedCatchScopes.class"))
            .arg(&source)
            .args(["--threads", "1"]),
    );
    let result = fs::read_to_string(source.join("NestedCatchScopes.java")).unwrap();
    for (method, catches) in [("run", 3), ("quiet", 2)] {
        let body = crate::common::source::method(&result, method);
        assert_eq!(body.matches("catch (").count(), catches, "{body}");
    }
    let probes = crate::common::source::method(&result, "probes");
    assert_eq!(probes.matches("\n        try {").count(), 2, "{probes}");
    let exits = crate::common::source::method(&result, "protectedExits");
    assert_eq!(exits.matches("catch (").count(), 2, "{exits}");
    assert!(
        exits.contains("finally {") && !exits.contains("label"),
        "{exits}"
    );
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-d"])
            .arg(&rebuilt)
            .arg(source.join("NestedCatchScopes.java")),
    );
    let actual = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&rebuilt)
            .arg("NestedCatchScopes"),
    );
    assert_eq!(actual, expected);
}
