//! Source arguments can own later versions without aliasing other arguments or
//! destroying values saved across writes, loop reentry, or exceptional edges.
use crate::common::checked;
use crate::common::source::method;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[test]
fn parameter_versions_preserve_original_values_and_exception_commits() {
    roundtrip("ParameterStorage", |text| {
        let cases = method(text, "cases");
        assert!(
            cases.contains("return ParameterStorage.step(40, 0);"),
            "{cases}"
        );
        assert!(!cases.contains("= p0;"), "{cases}");
        assert!(cases.contains("return p0;"), "{cases}");
        let reference = method(text, "reference");
        assert!(
            reference.contains("p1 = new ByteArrayInputStream("),
            "{reference}"
        );
        assert!(!reference.contains("(InputStream)"), "{reference}");
    });
}

#[test]
fn reference_phi_cycles_widen_monotonically_without_losing_null_or_array_values() {
    roundtrip("ReferenceCycle", |_| {});
}

fn roundtrip(name: &str, inspect: impl FnOnce(&str)) {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    let api = work.path().join("api.jar");
    let compile = |sources: &[PathBuf], output: &Path| {
        fs::create_dir_all(output).unwrap();
        checked(
            crate::common::compile_command()
                .args(["-g:none", "-d"])
                .arg(output)
                .args(sources),
        );
    };
    compile(
        &[PathBuf::from(format!("tests/fixtures/java/{name}.java"))],
        &original,
    );
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let text = fs::read_to_string(source.join(format!("{name}.java"))).unwrap();
    inspect(&text);
    let mut sources: Vec<_> = fs::read_dir(&source)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "java")
        })
        .collect();
    sources.sort();
    compile(&sources, &rebuilt);
    let execute = |classes: &Path| {
        checked(
            Command::new(crate::common::java())
                .arg("-cp")
                .arg(classes)
                .arg(name),
        )
        .stdout
    };
    let expected = execute(&original);
    assert_eq!(expected, execute(&rebuilt));

    if let Some(legacy) = crate::common::legacy_javac() {
        let output = work.path().join("legacy");
        fs::create_dir_all(&output).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&output)
                .args(&sources),
        );
        assert_eq!(expected, execute(&output));
    }
}
