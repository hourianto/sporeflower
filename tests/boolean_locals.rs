use crate::common;
use std::{fs, path::PathBuf, process::Command};

#[test]
fn predicate_local_components_recover_boolean_types_and_preserve_numeric_consumers() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original");
    let source = temp.path().join("source");
    let rebuilt = temp.path().join("rebuilt");
    let legacy = temp.path().join("legacy");
    let api = temp.path().join("api.jar");
    fs::create_dir_all(&original).unwrap();
    common::checked(
        common::compile_command()
            .args(["-g:none", "-d"])
            .arg(&original)
            .arg("tests/fixtures/java/BooleanLocals.java"),
    );
    let expected = common::execute(&original, "BooleanLocals");
    common::checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(original.join("BooleanLocals.class"))
            .arg(&source)
            .arg("--stubs")
            .arg(&api),
    );
    let text = fs::read_to_string(source.join("BooleanLocals.java")).unwrap();
    for name in [
        "pairedFlags",
        "equalityChain",
        "loopFlags",
        "descriptorFlags",
        "exactOne",
    ] {
        let method = common::source::method(&text, name);
        assert!(
            !method.contains("? 1 : 0") && !method.contains("? 0 : 1"),
            "{name} must retain predicate locals:\n{method}"
        );
    }
    assert!(
        common::source::method(&text, "loopFlags").contains("^="),
        "Boolean loop updates should retain compound assignment: {text}"
    );
    let sources: Vec<PathBuf> = common::sources(&source);
    common::compile(&sources, &rebuilt, None);
    assert_eq!(expected, common::execute(&rebuilt, "BooleanLocals"));
    if common::legacy_compile(&sources, &legacy, &api) {
        assert_eq!(expected, common::execute(&legacy, "BooleanLocals"));
    }
}
