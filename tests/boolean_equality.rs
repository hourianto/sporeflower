use crate::common::checked_stdout as checked;
use std::{fs, path::Path, process::Command};

fn boolean_descriptor(path: &Path) {
    let mut bytes = fs::read(path).unwrap();
    let matches: Vec<_> = bytes
        .windows(7)
        .enumerate()
        .filter_map(|(i, window)| (window == b"\0\x05(II)I").then_some(i))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "one shared constant-pool descriptor in {path:?}"
    );
    let offset = matches[0];
    bytes[offset + 3] = b'Z';
    bytes[offset + 4] = b'Z';
    fs::write(path, bytes).unwrap();
}

#[test]
fn boolean_equality_preserves_noncanonical_argument_slots() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original");
    let source = temp.path().join("source");
    let rebuilt = temp.path().join("rebuilt");
    let api = temp.path().join("api.jar");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-g:none", "-d"])
            .arg(&original)
            .args([
                "tests/fixtures/java/BooleanEquality.java",
                "tests/fixtures/java/BooleanEqualityDriver.java",
            ]),
    );
    for class in ["BooleanEquality", "BooleanEqualityDriver"] {
        boolean_descriptor(&original.join(format!("{class}.class")));
    }
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(original.join("BooleanEquality.class"))
            .arg(&source)
            .arg("--stubs")
            .arg(&api),
    );
    let text = fs::read_to_string(source.join("BooleanEquality.java")).unwrap();
    let method = |name| crate::common::source::method(&text, name);
    for name in ["equal", "different"] {
        let body = method(name);
        assert!(
            body.contains("p0 == p1") || body.contains("p0 != p1"),
            "{body}"
        );
        assert!(!body.contains("? 1 : 0)"), "{body}");
    }
    for name in ["exactlyTrue", "notExactlyTrue", "reversedTrue"] {
        let body = method(name);
        assert!(
            body.contains(" == true") || body.contains(" != true"),
            "{body}"
        );
        assert!(!body.contains("? 1 : 0)"), "{body}");
    }
    for name in ["normalizedTrue", "normalizedFalse"] {
        let body = method(name);
        assert_eq!(body.matches('?').count(), 1, "{body}");
        assert!(!body.contains("== 1") && !body.contains("!= 1"), "{body}");
    }
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .arg(source.join("BooleanEquality.java")),
    );
    fs::copy(
        original.join("BooleanEqualityDriver.class"),
        rebuilt.join("BooleanEqualityDriver.class"),
    )
    .unwrap();
    let run = |classes: &Path| {
        checked(
            Command::new(crate::common::java())
                .args(["-Xverify:all", "-cp"])
                .arg(classes)
                .arg("BooleanEqualityDriver"),
        )
    };
    assert_eq!(run(&original), run(&rebuilt));
    if let Some(legacy) = crate::common::legacy_javac() {
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&rebuilt)
                .arg(source.join("BooleanEquality.java")),
        );
        assert_eq!(run(&original), run(&rebuilt));
    }
}

/// Change only the reviewed fixture's descriptor constants and NEWARRAY's
/// component kind. Its instructions still load and pass arbitrary integer slots.
fn boolean_projection_descriptors(path: &Path) {
    fn u2(bytes: &[u8], offset: usize) -> usize {
        u16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap()) as usize
    }
    fn u4(bytes: &[u8], offset: usize) -> usize {
        u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize
    }
    let mut bytes = fs::read(path).unwrap();
    let mut offset = 10;
    let count = u2(&bytes, 8);
    let mut index = 1;
    let mut code_name = None;
    while index < count {
        let tag = bytes[offset];
        offset += 1;
        match tag {
            1 => {
                let length = u2(&bytes, offset);
                offset += 2;
                let text = &bytes[offset..offset + length];
                if text == b"Code" {
                    code_name = Some(index);
                }
                let replacement: Option<&[u8]> = match text {
                    b"I" => Some(b"Z"),
                    b"[B" => Some(b"[Z"),
                    b"(II)I" => Some(b"(ZI)Z"),
                    b"(I)I" => Some(b"(I)Z"),
                    b"([BI)I" => Some(b"([ZI)Z"),
                    b"(II)Ljava/lang/String;" => Some(b"(ZI)Ljava/lang/String;"),
                    b"([BI)Ljava/lang/String;" => Some(b"([ZI)Ljava/lang/String;"),
                    _ => None,
                };
                if let Some(replacement) = replacement {
                    assert_eq!(replacement.len(), length);
                    bytes[offset..offset + length].copy_from_slice(replacement);
                }
                offset += length;
            }
            3 | 4 | 9 | 10 | 11 | 12 | 18 => offset += 4,
            5 | 6 => {
                offset += 8;
                index += 1;
            }
            7 | 8 | 16 => offset += 2,
            15 => offset += 3,
            _ => panic!("unexpected authored constant-pool tag {tag}"),
        }
        index += 1;
    }
    offset += 6; // flags, this class, superclass
    let interfaces = u2(&bytes, offset);
    offset += 2 + interfaces * 2;
    for methods in [false, true] {
        let members = u2(&bytes, offset);
        offset += 2;
        for _ in 0..members {
            let attributes = u2(&bytes, offset + 6);
            offset += 8;
            for _ in 0..attributes {
                let name = u2(&bytes, offset);
                let length = u4(&bytes, offset + 2);
                offset += 6;
                if methods && Some(name) == code_name {
                    let code_length = u4(&bytes, offset + 4);
                    let start = offset + 8;
                    let instructions =
                        sporeflower::bytecode::decode(&bytes[start..start + code_length]).unwrap();
                    for instruction in instructions {
                        if instruction.opcode == 0xbc
                            && bytes[start + instruction.pc as usize + 1] == 8
                        {
                            bytes[start + instruction.pc as usize + 1] = 4;
                        }
                    }
                }
                offset += length;
            }
        }
    }
    fs::write(path, bytes).unwrap();
}

#[test]
fn boolean_source_choices_preserve_raw_slots_and_predicate_results() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original");
    let generated = temp.path().join("source");
    let api = temp.path().join("api.jar");
    fs::create_dir_all(&original).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-g:none", "-d"])
            .arg(&original)
            .args([
                "tests/fixtures/java/BooleanProjection.java",
                "tests/fixtures/java/BooleanProjectionDriver.java",
            ]),
    );
    for class in ["BooleanProjection", "BooleanProjectionDriver"] {
        boolean_projection_descriptors(&original.join(format!("{class}.class")));
    }
    let run = |classes: &Path| {
        checked(
            Command::new(crate::common::java())
                .args(["-Xint", "-Xverify:all", "-cp"])
                .arg(classes)
                .arg("BooleanProjectionDriver"),
        )
    };
    let expected = run(&original);
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(original.join("BooleanProjection.class"))
            .arg(&generated)
            .arg("--stubs")
            .arg(&api),
    );
    let source = generated.join("BooleanProjection.java");
    let text = fs::read_to_string(&source).unwrap();
    let method = |name: &str| {
        text.split(&format!(" {name}("))
            .nth(1)
            .unwrap()
            .split("\n    }")
            .next()
            .unwrap()
    };
    for name in [
        "parameter",
        "opposite",
        "local",
        "loop",
        "nested",
        "field",
        "array",
    ] {
        assert!(
            method(name).contains(" ? ") || method(name).contains("if ("),
            "raw result was normalized in {name}: {}",
            method(name)
        );
    }
    assert!(
        method("canonical").contains("&&"),
        "{}",
        method("canonical")
    );
    assert!(method("calls").contains('?'), "{}", method("calls"));
    assert!(
        method("canonicalLoop").contains("||"),
        "{}",
        method("canonicalLoop")
    );
    for name in [
        "eagerParameter",
        "eagerField",
        "eagerArray",
        "eagerXor",
        "eagerLoop",
    ] {
        let body = method(name);
        assert!(!body.contains(" ? 1 : 0"), "{name}: {body}");
        assert!(!body.contains(" != 0"), "{name}: {body}");
    }
    assert!(
        !method("numericSink").contains("boolean v"),
        "{}",
        method("numericSink")
    );
    for name in ["toggle", "alternateToggle"] {
        let body = method(name);
        assert_eq!(body.matches('?').count(), 1, "{name}: {body}");
        assert!(!body.contains("||"), "{name}: {body}");
        assert!(body.contains('&'), "{name}: raw mask must remain: {body}");
    }
    assert_eq!(
        method("alternateToggle").matches('&').count(),
        1,
        "the selected !selector is exactly true, but the raw saved value still needs its mask: {}",
        method("alternateToggle")
    );
    for (name, operator) in [
        ("compoundAnd", "&="),
        ("compoundOr", "|="),
        ("compoundXor", "^="),
    ] {
        assert!(method(name).contains(operator), "{name}: {}", method(name));
    }
    for name in [
        "predicateCapture",
        "predicateArrayCapture",
        "predicateCopyCapture",
        "predicateCatchCapture",
        "exactOneCapture",
        "catchExactOneCapture",
        "escapingCapture",
        "maskCapture",
        "predicateResultCapture",
        "exactOneResultCapture",
    ] {
        assert!(
            method(name).contains("? true : false"),
            "{name}: the captured predicate must keep its normalized value: {}",
            method(name)
        );
    }
    for (label, legacy) in std::iter::once(("modern", None))
        .chain(crate::common::legacy_javac().map(|compiler| ("legacy", Some(compiler))))
    {
        let rebuilt = temp.path().join(label);
        fs::create_dir_all(&rebuilt).unwrap();
        if let Some(legacy) = legacy {
            checked(
                crate::common::legacy_command(legacy, &api)
                    .arg("-d")
                    .arg(&rebuilt)
                    .arg(&source),
            );
        } else {
            checked(
                crate::common::compile_command()
                    .args(["-d"])
                    .arg(&rebuilt)
                    .arg(&source),
            );
        }
        fs::copy(
            original.join("BooleanProjectionDriver.class"),
            rebuilt.join("BooleanProjectionDriver.class"),
        )
        .unwrap();
        let actual = run(&rebuilt);
        assert_eq!(
            String::from_utf8_lossy(&expected),
            String::from_utf8_lossy(&actual),
            "{label} must preserve raw return bits and predicate normalization\n{text}"
        );
    }
}

#[test]
fn partial_input_keeps_public_boolean_normalization_with_an_internal_caller() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original");
    let source = temp.path().join("source");
    let rebuilt = temp.path().join("rebuilt");
    let api = temp.path().join("api.jar");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    let boundary = temp.path().join("Boundary.java");
    let driver = temp.path().join("Driver.java");
    fs::write(&boundary, "public class Boundary { public static int normalize(int value) { return value != 0 ? 1 : 0; } static int internalCaller() { return normalize(0); } }").unwrap();
    fs::write(&driver, "public class Driver { public static void main(String[] args) { System.out.println(Boundary.normalize(2)); } }").unwrap();
    checked(
        crate::common::compile_command()
            .args(["-g:none", "-d"])
            .arg(&original)
            .arg(&boundary)
            .arg(&driver),
    );
    for name in ["Boundary", "Driver"] {
        let path = original.join(format!("{name}.class"));
        let mut bytes = fs::read(&path).unwrap();
        let offsets: Vec<_> = bytes
            .windows(6)
            .enumerate()
            .filter_map(|(offset, window)| (window == b"\0\x04(I)I").then_some(offset + 2))
            .collect();
        assert_eq!(offsets.len(), 1, "one method descriptor in {name}");
        bytes[offsets[0]..offsets[0] + 4].copy_from_slice(b"(Z)Z");
        fs::write(path, bytes).unwrap();
    }
    // The external raw caller is deliberately omitted from decompiler input.
    // An observed normalize(false) call must not close this public boundary.
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(original.join("Boundary.class"))
            .arg(&source)
            .arg("--stubs")
            .arg(&api),
    );
    let run = |classes: &Path| {
        checked(
            Command::new(crate::common::java())
                .args(["-Xverify:all", "-cp"])
                .arg(classes)
                .arg("Driver"),
        )
    };
    let expected = run(&original);
    assert_eq!(expected, b"1\n");
    checked(
        crate::common::compile_command()
            .arg("-d")
            .arg(&rebuilt)
            .arg(source.join("Boundary.java")),
    );
    fs::copy(original.join("Driver.class"), rebuilt.join("Driver.class")).unwrap();
    assert_eq!(run(&rebuilt), expected);
    if let Some(legacy) = crate::common::legacy_javac() {
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&rebuilt)
                .arg(source.join("Boundary.java")),
        );
        assert_eq!(run(&rebuilt), expected);
    }
}
