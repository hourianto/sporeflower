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
fn null_casts_preserve_overloads_and_nonnull_failures() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("NullCasts.java");
    fs::write(&input, r#"
public class NullCasts {
    static int choose(String value) { return 1; }
    static int choose(Object value) { return 2; }
    static int choose(int[] value) { return 3; }
    static int[] array(boolean stop) {
        Object value = null;
        if (stop) return (int[]) value;
        return new int[] { 7 };
    }
    static int argument(boolean stop) {
        Object value = null;
        if (stop) return choose((String) value);
        return choose(value);
    }
    static int arrays(boolean stop) {
        Object value = null;
        if (stop) return choose((int[]) value);
        return choose(value);
    }
    static int nonnull(Object value) { return choose((String) value); }
    public static void main(String[] args) {
        System.out.println(array(true));
        System.out.println(array(false)[0]);
        System.out.println(argument(true) + ":" + argument(false));
        System.out.println(arrays(true) + ":" + arrays(false));
        System.out.println(nonnull(null) + ":" + nonnull("test"));
        try { nonnull(new int[0]); } catch (ClassCastException expected) { System.out.println("cast"); }
    }
}
"#).unwrap();
    let original = work.path().join("original");
    fs::create_dir_all(&original).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-g:none", "-d"])
            .arg(&original)
            .arg(&input),
    );
    let generated = work.path().join("generated");
    let api = work.path().join("api.jar");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&generated)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let text = fs::read_to_string(generated.join("NullCasts.java")).unwrap();
    for name in ["array", "argument", "arrays"] {
        let body = method(&text, name);
        assert!(!body.contains(") (Object) null"), "{body}");
    }
    assert!(
        method(&text, "argument").contains("(String) null"),
        "{text}"
    );
    assert!(method(&text, "arrays").contains("(int[]) null"), "{text}");
    assert!(method(&text, "nonnull").contains("(String) p0"), "{text}");
    let files = crate::common::sources(&generated);
    let rebuilt = work.path().join("rebuilt");
    crate::common::compile(&files, &rebuilt, None);
    let expected = crate::common::execute(&original, "NullCasts");
    assert_eq!(expected, crate::common::execute(&rebuilt, "NullCasts"));
    let legacy = work.path().join("legacy");
    if crate::common::legacy_compile(&files, &legacy, &api) {
        assert_eq!(expected, crate::common::execute(&legacy, "NullCasts"));
    }
}

#[test]
fn mask_spelling_and_data_tables_preserve_width_order_and_narrow_values() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("MaskSpelling.java");
    fs::write(&input, r#"
public class MaskSpelling {
    static int trace;
    static long field;
    static int step(int value) { trace = trace * 31 + value; return value; }
    static boolean check(int value) { trace = trace * 31 + value; return value > 0; }
    static int packed(int a, int b) { return (step(a) & 255) | ((step(b) << 8) & 65280); }
    static int shift(int a, int b) { return (a + 1) << (b + 4); }
    static int negative(int value) { return value & -268435457; }
    static long wide(long value) { return value & -16777216L; }
    static long widen(int value) { return value & 4294967295L; }
    static long update(long value) { field = value; field &= -16777216L; field ^= 4294967295L; return field; }
    static boolean logical(int a, int b) { return check(a) || (check(b) && check(a + b)); }
    static byte[] bytes() { return new byte[] { 0, 127, -128, 1, -1 }; }
    static short[] shorts() { return new short[] { 0, 32767, -32768, 1, -1 }; }
    static void bytesAt(byte[] values, int offset) { values[offset] = 127; values[offset + 1] = -128; }
    static void shortsAt(short[] values, int offset) { values[offset] = 32767; values[offset + 1] = -32768; }
    static int byteCase(byte value) {
        switch (value) { case 127: return 1; case -128: return 2; default: return 3; }
    }
    static int shortCase(short value) {
        switch (value) { case 32767: return 1; case -32768: return 2; default: return 3; }
    }
    static byte limit() { return 127; }
    public static void main(String[] args) {
        int[] values = { -2147483648, -1, 0, 1, 2147483647 };
        for (int i = 0; i < values.length; i++) {
            for (int j = 0; j < values.length; j++) {
                int a = values[i], b = values[j];
                trace = 0;
                System.out.println(packed(a, b) + ":" + shift(a, b) + ":" + negative(a));
                System.out.println(wide((long) a * b) + ":" + widen(a) + ":" + update((long) a * b));
                System.out.println(logical(a, b) + ":" + trace);
            }
        }
        byte[] bytes = bytes();
        short[] shorts = shorts();
        for (int i = 0; i < bytes.length; i++) System.out.println(bytes[i] + ":" + shorts[i]);
        bytesAt(bytes, 1);
        shortsAt(shorts, 2);
        for (int i = 0; i < bytes.length; i++) System.out.println(byteCase(bytes[i]) + ":" + shortCase(shorts[i]));
        System.out.println(limit());
    }
}
"#).unwrap();
    let original = work.path().join("original");
    compile(&[input], &original);
    let generated = work.path().join("generated");
    let api = work.path().join("api.jar");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&generated)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let text = fs::read_to_string(generated.join("MaskSpelling.java")).unwrap();
    for (name, spelling) in [
        ("packed", "& 0xff) |"),
        ("packed", "<< 8) & 0xff00)"),
        ("shift", "(a + 1) << (b + 4)"),
        ("negative", "& 0xefffffff"),
        ("wide", "& 0xffffffffff000000L"),
        ("widen", "& 0xffffffffL"),
        ("update", "&= 0xffffffffff000000L"),
        ("update", "^= 0xffffffffL"),
        ("limit", "Byte.MAX_VALUE"),
        ("bytesAt", "values[offset] = 127;"),
        ("bytesAt", "values[offset + 1] = -128;"),
        ("shortsAt", "values[offset] = 32767;"),
        ("shortsAt", "values[offset + 1] = -32768;"),
        ("byteCase", "case 127:"),
        ("byteCase", "case -128:"),
        ("shortCase", "case 32767:"),
        ("shortCase", "case -32768:"),
    ] {
        assert!(
            method(&text, name).contains(spelling),
            "missing {spelling}:\n{text}"
        );
    }
    for (name, expected) in [
        ("bytes", "{ 0, 127, -128, 1, -1 }"),
        ("shorts", "{ 0, 32767, -32768, 1, -1 }"),
    ] {
        let body = method(&text, name);
        assert!(body.contains(expected), "{body}");
        assert!(
            !body.contains(".MAX_VALUE") && !body.contains(".MIN_VALUE"),
            "{body}"
        );
    }
    let files = crate::common::sources(&generated);
    let rebuilt = work.path().join("rebuilt");
    crate::common::compile(&files, &rebuilt, None);
    let expected = crate::common::execute(&original, "MaskSpelling");
    assert_eq!(expected, crate::common::execute(&rebuilt, "MaskSpelling"));
    let legacy = work.path().join("legacy");
    if crate::common::legacy_compile(&files, &legacy, &api) {
        assert_eq!(expected, crate::common::execute(&legacy, "MaskSpelling"));
    }
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
fn integral_idioms_preserve_width_effects_and_named_masks() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &["tests/fixtures/java/IntegralIdioms.java".into()],
        &original,
    );
    crate::common::decompile(&original, &source);
    let text = fs::read_to_string(source.join("IntegralIdioms.java")).unwrap();
    for name in [
        "complement",
        "complementLeft",
        "complementLong",
        "complementWidened",
        "complementByte",
        "complementChar",
        "complementSum",
    ] {
        let body = method(&text, name);
        assert!(
            body.contains('~') && !body.contains("^ -1"),
            "{name}: {body}"
        );
    }
    assert!(
        method(&text, "complementSum")
            .contains("~(IntegralIdioms.step(left) + IntegralIdioms.step(right))"),
        "{text}"
    );
    assert!(
        method(&text, "doubleComplement").contains("return value;"),
        "{text}"
    );
    assert!(
        method(&text, "partialLongMask").contains("^ 0xffffffffL"),
        "{text}"
    );
    assert!(method(&text, "partialIntMask").contains("^ 0xff"), "{text}");
    assert!(method(&text, "boolXor").contains("left ^ right"), "{text}");
    assert!(!method(&text, "identities").contains("& -1"), "{text}");
    assert!(!method(&text, "identities").contains("| 0"), "{text}");
    assert!(!method(&text, "wideIdentity").contains("^ 0"), "{text}");
    assert!(method(&text, "eager").contains("& 0"), "{text}");
    assert!(
        method(&text, "zeroMinus").contains("return -value;"),
        "{text}"
    );
    assert!(
        method(&text, "subtractNegated").contains("step(left) + IntegralIdioms.step(right)"),
        "{text}"
    );
    assert!(
        method(&text, "addNegated").contains("step(left) - IntegralIdioms.step(right)"),
        "{text}"
    );
    assert!(
        method(&text, "doubleNegated").contains("return value;"),
        "{text}"
    );
    assert!(
        method(&text, "floatingZeroMinus").contains("0.0F - value"),
        "{text}"
    );
    compile(&crate::common::sources(&source), &rebuilt);
    assert_eq!(
        crate::common::execute(&original, "IntegralIdioms"),
        crate::common::execute(&rebuilt, "IntegralIdioms")
    );

    let mappings = work.path().join("masks.map");
    fs::write(
        &mappings,
        r#"
package readable;
@FlagDomain interface Bits {}
class IntegralIdioms /* was IntegralIdioms */ {
    @DomainValue(Bits.class) static final int ALL /* was ALL */;
    @Flags(Bits.class) int mapped(@Flags(Bits.class) int value) /* was mapped */;
}
"#,
    )
    .unwrap();
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .arg("--mappings")
            .arg(&mappings),
    );
    let mapped = fs::read_to_string(source.join("readable/IntegralIdioms.java")).unwrap();
    assert!(method(&mapped, "mapped").contains("^ ALL"), "{mapped}");
    compile(&crate::common::sources(&source), &rebuilt);
    assert_eq!(
        crate::common::execute(&original, "IntegralIdioms"),
        crate::common::execute(&rebuilt, "readable.IntegralIdioms")
    );
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
