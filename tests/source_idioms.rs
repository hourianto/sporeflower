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
    assert!(
        method(&text, "minimum").contains("+ Integer.MIN_VALUE"),
        "{text}"
    );
    assert!(
        method(&text, "minimumLong").contains("- Long.MIN_VALUE"),
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

#[test]
fn unicode_literals_are_readable_and_preserve_utf16_units() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    crate::common::compile(
        &["tests/fixtures/java/UnicodeLiterals.java".into()],
        &original,
        None,
    );
    crate::common::decompile(&original, &source);
    let text = fs::read_to_string(source.join("UnicodeLiterals.java")).unwrap();
    for (name, spelling) in [
        (
            "translated",
            "\"Crème brûlée — Ελληνικά Русский العربية 中文 日本語 한글\"",
        ),
        ("supplementary", "\"😀 𐐀 𝄞\""),
        ("combining", "\"e\u{0301}\""),
        ("accented", "'é'"),
        ("quoted", "'\\''"),
        ("backslash", "'\\\\'"),
        ("control", "'\\000'"),
        (
            "controls",
            r#""\0007\001\007\b\t\n\013\f\r\037\177\200\237""#,
        ),
        (
            "hidden",
            r#""\240\255\u200b\u2028\u2029\u202e\ue000\uffff\udb40\udc01\udb80\udc00\u0378\ud83f\udffe""#,
        ),
        ("surrogates", r#""\ud800x\udc00\udbff𐀀\udfff""#),
        (
            "escapeText",
            r#""\"'\\u000a|\\\\u0041|\\uuuu000d|\ud800\\u005c|\\\ud800""#,
        ),
    ] {
        assert!(method(&text, name).contains(spelling), "{name}:\n{text}");
    }
    assert!(text.contains("LETTER = 'Ж'"), "{text}");
    assert!(text.contains("SURROGATE = '\\ud800'"), "{text}");

    crate::common::compile(&crate::common::sources(&source), &rebuilt, None);
    assert_eq!(
        crate::common::execute(&original, "UnicodeLiterals"),
        crate::common::execute(&rebuilt, "UnicodeLiterals"),
        "every UTF-16 code unit must survive the round trip"
    );
}
