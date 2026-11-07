//! Source spelling follows types and lexical bindings, not textual patterns.
use crate::common::checked_stdout as checked;
use crate::common::source::method;
use sporeflower::{
    input::Input,
    java::{JavaOptions, decompile_class},
    naming::{NamePlan, RenameOptions},
    program::Program,
};
use std::{fs, path::Path, process::Command};

fn compile(files: &[std::path::PathBuf], output: &Path) {
    fs::create_dir_all(output).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-g", "-d"])
            .arg(output)
            .args(files),
    );
}

#[test]
fn typed_literals_and_names_preserve_numeric_boundaries_and_bindings() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(&["tests/fixtures/java/SourceIdioms.java".into()], &original);
    let mut input = Input::read(&original).unwrap().into_classes();
    for class in &mut input {
        for method in &mut class.methods {
            if matches!(method.name.as_str(), "unnamedCatch" | "sharedCatch") {
                method.code.as_mut().unwrap().local_variables.clear();
            }
        }
    }
    let api = sporeflower::api::ApiCatalog::bundled()
        .unwrap()
        .select(&input, &Default::default())
        .unwrap();
    let program = Program::new(&input, &api.classes).unwrap();
    let names = NamePlan::build(&program, &RenameOptions::default()).unwrap();
    fs::create_dir_all(&source).unwrap();
    let mut files = Vec::new();
    for id in 0..program.own_count() {
        let output = decompile_class(&program, &names, id, &JavaOptions::default()).unwrap();
        assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
        let path = source.join(format!("{}.java", names.class_name(id)));
        fs::write(&path, output.source).unwrap();
        files.push(path);
    }
    let text = fs::read_to_string(source.join("SourceIdioms.java")).unwrap();
    assert!(
        text.contains("beforeInitialization = SourceIdioms.initialized"),
        "{text}"
    );
    assert!(
        method(&text, "slash").contains("charAt(0) != '/'"),
        "{text}"
    );
    assert!(method(&text, "character").contains("'/'"), "{text}");
    assert!(method(&text, "character").contains("'\\uffff'"), "{text}");
    assert!(
        method(&text, "outsideCharacter").contains("65536"),
        "{text}"
    );
    assert!(method(&text, "integer").contains("!= 47"), "{text}");
    assert!(method(&text, "addNegative").contains("value - 7"), "{text}");
    assert!(
        method(&text, "subtractNegative").contains("value + 7"),
        "{text}"
    );
    assert!(
        method(&text, "addNegativeLong").contains("value - 7L"),
        "{text}"
    );
    assert!(method(&text, "minimum").contains("+ -2147483648"), "{text}");
    assert!(
        method(&text, "minimumLong").contains("- -9223372036854775808L"),
        "{text}"
    );
    assert!(method(&text, "floating").contains("+ -1.0F"), "{text}");
    assert!(method(&text, "update").contains("-= 7"), "{text}");
    assert!(
        method(&text, "updateField").contains("this.value -= 7"),
        "{text}"
    );
    assert!(method(&text, "updateArray").contains("[0] -= 7"), "{text}");
    assert!(
        method(&text, "ownField").contains("return total;"),
        "{text}"
    );
    assert!(
        method(&text, "shadowed").contains("SourceIdioms.total + total"),
        "{text}"
    );
    assert!(
        method(&text, "inheritedField").contains("SourceIdiomsBase.total"),
        "{text}"
    );
    assert!(
        method(&text, "namedCatch").contains("ArithmeticException cause"),
        "{text}"
    );
    assert!(
        !text.contains("$caught") && !text.contains("$exception"),
        "{text}"
    );
    assert!(
        method(&text, "unnamedCatch").contains("ArithmeticException e"),
        "{text}"
    );
    compile(&files, &rebuilt);
    let execute = |path: &Path| {
        checked(
            Command::new(crate::common::java())
                .arg("-cp")
                .arg(path)
                .arg("SourceIdioms"),
        )
    };
    assert_eq!(execute(&original), execute(&rebuilt));
}
