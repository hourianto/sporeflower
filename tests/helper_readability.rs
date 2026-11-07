use crate::common::source::method;
use crate::common::{checked, compile, files, legacy_compile};
use std::{fs, path::Path, process::Command};

fn execute(classes: &Path, main: &str, args: &[&str]) -> Vec<u8> {
    checked(
        Command::new(crate::common::java())
            .args(["-Xverify:all", "-cp"])
            .arg(classes)
            .arg(main)
            .args(args),
    )
    .stdout
}

#[test]
fn subtype_array_arguments_remain_expressions_without_moving_constructor_effects() {
    let work = tempfile::tempdir().unwrap();
    let root = Path::new("tests/fixtures/helper_readability");
    let support_sources = files(&root.join("support"), "java");
    let library = work.path().join("library");
    let original = work.path().join("original");
    let source = work.path().join("source");
    let api = work.path().join("api.jar");
    compile(&support_sources, &library, None);
    compile(
        &[root.join("AllocationExpressions.java")],
        &original,
        Some(&library),
    );
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .args(["--threads", "1", "--classpath"])
            .arg(&library)
            .arg("--stubs")
            .arg(&api),
    );
    let emitted = fs::read_to_string(source.join("AllocationExpressions.java")).unwrap();
    assert!(
        !emitted.contains("$sf$argument") && !emitted.contains("$sf$Arguments"),
        "{emitted}"
    );
    for initializer in ["new Base[] {", "new Face[] {", "new Base[][] {"] {
        assert!(emitted.contains(initializer), "{emitted}");
    }
    assert!(
        emitted.contains("[0] ="),
        "unsafe array stores must remain stores: {emitted}"
    );
    // The library stays external during decompilation so its initialization
    // cannot be assumed quiet. It is included unchanged in both executions.
    compile(&support_sources, &original, None);
    let mut sources = files(&source, "java");
    sources.extend(support_sources);
    let rebuilt = work.path().join("rebuilt");
    compile(&sources, &rebuilt, None);
    let legacy = work.path().join("legacy");
    let has_legacy = legacy_compile(&sources, &legacy, &api);
    for mode in 0..7 {
        let mode = mode.to_string();
        let expected = execute(&original, "AllocationExpressions", &[&mode]);
        assert_eq!(
            expected,
            execute(&rebuilt, "AllocationExpressions", &[&mode]),
            "{mode}\n{emitted}"
        );
        if has_legacy {
            assert_eq!(
                expected,
                execute(&legacy, "AllocationExpressions", &[&mode]),
                "legacy {mode}\n{emitted}"
            );
        }
        let trace = String::from_utf8(expected).unwrap();
        if mode == "1" {
            assert!(!trace.contains("title;"), "{trace}");
            assert!(!trace.contains("indexed-array;"), "{trace}");
        }
        assert!(trace.contains("java.lang.ArrayStoreException"), "{trace}");
    }
}

fn relocate(bytes: &[u8]) -> Vec<u8> {
    let count = u16::from_be_bytes([bytes[8], bytes[9]]) as usize;
    let mut result = bytes[..10].to_vec();
    let mut at = 10;
    let mut index = 1;
    while index < count {
        let tag = bytes[at];
        at += 1;
        result.push(tag);
        if tag == 1 {
            let size = u16::from_be_bytes([bytes[at], bytes[at + 1]]) as usize;
            at += 2;
            let value = std::str::from_utf8(&bytes[at..at + size])
                .unwrap()
                .replace("orig/ResourceNames", "ResourceNames")
                .replace("orig.ResourceNames", "ResourceNames");
            result.extend_from_slice(&(value.len() as u16).to_be_bytes());
            result.extend_from_slice(value.as_bytes());
            at += size;
        } else {
            let size = match tag {
                3 | 4 | 9 | 10 | 11 | 12 | 17 | 18 => 4,
                5 | 6 => {
                    index += 1;
                    8
                }
                7 | 8 | 16 | 19 | 20 => 2,
                15 => 3,
                _ => panic!("constant tag {tag}"),
            };
            result.extend_from_slice(&bytes[at..at + size]);
            at += size;
        }
        index += 1;
    }
    result.extend_from_slice(&bytes[at..]);
    result
}

fn add_resources(classes: &Path) {
    fs::create_dir_all(classes.join("client")).unwrap();
    fs::write(classes.join("token.txt"), [17]).unwrap();
    fs::write(classes.join("client/token.txt"), [23]).unwrap();
    fs::write(classes.join("nulltoken.txt"), [31]).unwrap();
}

#[test]
fn absolute_resource_names_keep_direct_calls_and_relative_names_keep_relocation() {
    let work = tempfile::tempdir().unwrap();
    let root = Path::new("tests/fixtures/helper_readability/resources");
    let compiled = work.path().join("compiled");
    let original = work.path().join("original");
    let source = work.path().join("source");
    let api = work.path().join("api.jar");
    compile(&files(root, "java"), &compiled, None);
    fs::create_dir_all(original.join("client")).unwrap();
    for (from, to) in [
        ("orig/ResourceNames.class", "ResourceNames.class"),
        ("client/ResourceDriver.class", "client/ResourceDriver.class"),
    ] {
        fs::write(
            original.join(to),
            relocate(&fs::read(compiled.join(from)).unwrap()),
        )
        .unwrap();
    }
    add_resources(&original);
    let expected = execute(&original, "client.ResourceDriver", &[]);
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .args(["--threads", "1", "--stubs"])
            .arg(&api),
    );
    let emitted = fs::read_to_string(source.join("defaultpkg/ResourceNames.java")).unwrap();
    for direct in [
        "absolute",
        "concatenated",
        "characterPrefix",
        "chosen",
        "nullName",
    ] {
        assert!(
            !method(&emitted, direct).contains("$sf$resource"),
            "{emitted}"
        );
    }
    for relative in ["relative", "dynamic", "nullablePrefix", "mutableName"] {
        assert!(
            method(&emitted, relative).contains("$sf$resource"),
            "{emitted}"
        );
    }
    let sources = files(&source, "java");
    let rebuilt = work.path().join("rebuilt");
    compile(&sources, &rebuilt, None);
    add_resources(&rebuilt);
    assert_eq!(
        expected,
        execute(&rebuilt, "client.ResourceDriver", &[]),
        "{emitted}"
    );
    let legacy = work.path().join("legacy");
    if legacy_compile(&sources, &legacy, &api) {
        add_resources(&legacy);
        assert_eq!(
            expected,
            execute(&legacy, "client.ResourceDriver", &[]),
            "{emitted}"
        );
    }
}
