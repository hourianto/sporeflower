use crate::common::{checked, compile};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[test]
fn external_new_precedes_independent_effects_and_keeps_exception_coverage() {
    let work = tempfile::tempdir().unwrap();
    let library = work.path().join("library");
    let original = work.path().join("original");
    let source = work.path().join("source");
    let recompiled = work.path().join("recompiled");
    let support = [
        PathBuf::from("tests/fixtures/allocation_lib/AllocationState.java"),
        PathBuf::from("tests/fixtures/allocation_lib/AllocationExternal.java"),
    ];
    compile(&support, &library, None);
    compile(
        &[PathBuf::from("tests/fixtures/java/AllocationClient.java")],
        &original,
        Some(&library),
    );
    let path = original.join("AllocationClient.class");
    let mut bytes = fs::read(&path).unwrap();
    let class = sporeflower::classfile::ClassFile::parse(&bytes).unwrap();
    for name in ["run", "guarded"] {
        let code = &class
            .methods
            .iter()
            .find(|method| method.name == name)
            .unwrap()
            .code
            .as_ref()
            .unwrap()
            .bytes;
        let offset = bytes
            .windows(code.len())
            .position(|window| window == code)
            .unwrap();
        let instructions = sporeflower::bytecode::decode(code).unwrap();
        let allocation = instructions
            .iter()
            .position(|instruction| instruction.opcode == 0xbb)
            .unwrap();
        let begin = instructions[allocation].pc as usize;
        assert_eq!(instructions[allocation + 1].opcode, 0x59);
        let end = instructions[allocation + 2].pc as usize;
        let reordered: Vec<_> = code[begin..end]
            .iter()
            .chain(&code[..begin])
            .copied()
            .collect();
        bytes[offset..offset + end].copy_from_slice(&reordered);
    }
    fs::write(path, bytes).unwrap();
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .args(["--no-api", "--threads", "1", "--classpath"])
            .arg(&library),
    );
    let emitted = fs::read_to_string(source.join("AllocationClient.java")).unwrap();
    assert!(
        emitted.contains("new AllocationExternal(AllocationClient.$sf$argument_"),
        "{emitted}"
    );
    assert!(
        emitted.contains("throws IOException"),
        "checked helper effects must remain visible: {emitted}"
    );
    compile(
        &[source.join("AllocationClient.java")],
        &recompiled,
        Some(&library),
    );
    let driver = PathBuf::from("tests/fixtures/allocation_lib/AllocationDriver.java");
    for classes in [&original, &recompiled] {
        for name in ["AllocationState.class", "AllocationExternal.class"] {
            fs::copy(library.join(name), classes.join(name)).unwrap();
        }
        compile(std::slice::from_ref(&driver), classes, Some(classes));
    }
    for method in ["run", "guarded"] {
        for mode in 0..5 {
            let execute = |classes: &Path| {
                checked(
                    Command::new(crate::common::java())
                        .arg("-cp")
                        .arg(classes)
                        .arg("AllocationDriver")
                        .arg(mode.to_string())
                        .arg(method),
                )
                .stdout
            };
            let expected = execute(&original);
            let actual = execute(&recompiled);
            assert_eq!(
                String::from_utf8_lossy(&actual),
                String::from_utf8_lossy(&expected),
                "{method} mode {mode}"
            );
            if mode == 1 {
                assert!(!String::from_utf8_lossy(&actual).contains("touch;"));
            }
        }
    }
}

#[test]
fn multiple_arguments_export_partial_locals_on_helper_and_constructor_failures() {
    let work = tempfile::tempdir().unwrap();
    let library = work.path().join("library");
    let original = work.path().join("original");
    let source = work.path().join("source");
    let recompiled = work.path().join("recompiled");
    let support = [
        PathBuf::from("tests/fixtures/allocation_lib/AllocationState.java"),
        PathBuf::from("tests/fixtures/allocation_lib/AllocationResource.java"),
        PathBuf::from("tests/fixtures/allocation_lib/AllocationWrapper.java"),
    ];
    compile(&support, &library, None);
    compile(
        &[PathBuf::from(
            "tests/fixtures/java/AllocationLiveClient.java",
        )],
        &original,
        Some(&library),
    );
    let path = original.join("AllocationLiveClient.class");
    let mut bytes = fs::read(&path).unwrap();
    let class = sporeflower::classfile::ClassFile::parse(&bytes).unwrap();
    // Old verification reconstructs frames after NEW moves across branch labels.
    bytes[6..8].copy_from_slice(&49u16.to_be_bytes());
    for name in ["run", "branch", "loop", "owned", "innerPlain"] {
        let code = &class
            .methods
            .iter()
            .find(|method| method.name == name)
            .unwrap()
            .code
            .as_ref()
            .unwrap()
            .bytes;
        let instructions = sporeflower::bytecode::decode(code).unwrap();
        let open = instructions
            .iter()
            .position(|instruction| instruction.opcode == 0xb8)
            .unwrap();
        let start = instructions[open - 1].pc as usize;
        let allocation = instructions.iter().position(|instruction| {
            instruction.opcode == 0xbb && matches!(instruction.operand, sporeflower::bytecode::Operand::Constant(index) if class.constant_pool.class_name(index).unwrap() == "AllocationWrapper")
        }).unwrap();
        let begin = instructions[allocation].pc as usize;
        assert_eq!(instructions[allocation + 1].opcode, 0x59);
        let end = instructions[allocation + 2].pc as usize;
        let reordered: Vec<_> = code[begin..end]
            .iter()
            .chain(&code[start..begin])
            .copied()
            .collect();
        let offset = bytes
            .windows(code.len())
            .position(|window| window == code)
            .unwrap();
        bytes[offset + start..offset + end].copy_from_slice(&reordered);
        let max_stack = u16::from_be_bytes(bytes[offset - 8..offset - 6].try_into().unwrap());
        bytes[offset - 8..offset - 6].copy_from_slice(&(max_stack + 2).to_be_bytes());
        if matches!(name, "owned" | "innerPlain") {
            // The inner catch moves with its argument instructions. The outer
            // catch already enclosed NEW and continues to cover it at entry.
            let table = offset + code.len();
            let count = u16::from_be_bytes(bytes[table..table + 2].try_into().unwrap()) as usize;
            for index in 0..count {
                let entry = table + 2 + 8 * index;
                let from = u16::from_be_bytes(bytes[entry..entry + 2].try_into().unwrap()) as usize;
                let to =
                    u16::from_be_bytes(bytes[entry + 2..entry + 4].try_into().unwrap()) as usize;
                let handler =
                    u16::from_be_bytes(bytes[entry + 4..entry + 6].try_into().unwrap()) as usize;
                let shift = end - begin;
                if from >= start && to <= begin {
                    bytes[entry..entry + 2].copy_from_slice(&((from + shift) as u16).to_be_bytes());
                    bytes[entry + 2..entry + 4]
                        .copy_from_slice(&((to + shift) as u16).to_be_bytes());
                }
                if handler >= start && handler < begin {
                    bytes[entry + 4..entry + 6]
                        .copy_from_slice(&((handler + shift) as u16).to_be_bytes());
                }
            }
        }
    }
    fs::write(path, bytes).unwrap();
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .args(["--no-api", "--threads", "1", "--classpath"])
            .arg(&library),
    );
    let emitted = fs::read_to_string(source.join("AllocationLiveClient.java")).unwrap();
    assert!(
        emitted.contains("$sf$Arguments"),
        "fixture must use typed live-output transport: {emitted}"
    );
    compile(
        &[source.join("AllocationLiveClient.java")],
        &recompiled,
        Some(&library),
    );
    let driver = PathBuf::from("tests/fixtures/allocation_lib/AllocationLiveDriver.java");
    for classes in [&original, &recompiled] {
        for name in [
            "AllocationState.class",
            "AllocationResource.class",
            "AllocationWrapper.class",
        ] {
            fs::copy(library.join(name), classes.join(name)).unwrap();
        }
        compile(std::slice::from_ref(&driver), classes, Some(classes));
    }
    for (method, limit) in [
        ("run", 6),
        ("branch", 7),
        ("loop", 8),
        ("owned", 6),
        ("innerPlain", 6),
    ] {
        for mode in 0..limit {
            let execute = |classes: &Path| {
                checked(
                    Command::new(crate::common::java())
                        .arg("-cp")
                        .arg(classes)
                        .arg("AllocationLiveDriver")
                        .arg(mode.to_string())
                        .arg(method),
                )
                .stdout
            };
            let expected = execute(&original);
            let actual = execute(&recompiled);
            assert_eq!(
                String::from_utf8_lossy(&actual),
                String::from_utf8_lossy(&expected),
                "{method} mode {mode}\n{emitted}"
            );
            if mode == 3 || mode == 5 {
                assert!(String::from_utf8_lossy(&actual).contains("closeResource;null;"));
            }
        }
    }
}

#[test]
fn nested_external_allocations_keep_outer_and_inner_initialization_order() {
    let work = tempfile::tempdir().unwrap();
    let library = work.path().join("library");
    let original = work.path().join("original");
    let source = work.path().join("source");
    let recompiled = work.path().join("recompiled");
    compile(
        &[
            PathBuf::from("tests/fixtures/allocation_lib/AllocationState.java"),
            PathBuf::from("tests/fixtures/allocation_lib/AllocationInner.java"),
            PathBuf::from("tests/fixtures/allocation_lib/AllocationOuter.java"),
        ],
        &library,
        None,
    );
    compile(
        &[PathBuf::from(
            "tests/fixtures/java/AllocationNestedClient.java",
        )],
        &original,
        Some(&library),
    );
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .args(["--no-api", "--threads", "1", "--classpath"])
            .arg(&library),
    );
    let emitted = fs::read_to_string(source.join("AllocationNestedClient.java")).unwrap();
    assert!(
        !emitted.contains("$sf$Arguments"),
        "stable constructor arguments need no carrier: {emitted}"
    );
    compile(
        &[source.join("AllocationNestedClient.java")],
        &recompiled,
        Some(&library),
    );
    let driver = PathBuf::from("tests/fixtures/allocation_lib/AllocationNestedDriver.java");
    for classes in [&original, &recompiled] {
        for name in [
            "AllocationState.class",
            "AllocationInner.class",
            "AllocationOuter.class",
        ] {
            fs::copy(library.join(name), classes.join(name)).unwrap();
        }
        compile(std::slice::from_ref(&driver), classes, Some(classes));
    }
    for mode in 0..7 {
        let execute = |classes: &Path| {
            checked(
                Command::new(crate::common::java())
                    .arg("-cp")
                    .arg(classes)
                    .arg("AllocationNestedDriver")
                    .arg(mode.to_string()),
            )
            .stdout
        };
        let expected = execute(&original);
        let actual = execute(&recompiled);
        assert_eq!(
            String::from_utf8_lossy(&actual),
            String::from_utf8_lossy(&expected),
            "mode {mode}\n{emitted}"
        );
    }
}
