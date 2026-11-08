use crate::common::checked;
use std::path::Path;
use std::process::Command;

#[test]
fn initialization_triggers_replay_original_blocks_after_select_recovery() {
    use sporeflower::{bytecode, classfile::ClassFile};
    use std::fs;
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    crate::common::compile(
        &["tests/fixtures/java/InitializationPlacement.java".into()],
        &original,
        None,
    );
    let path = original.join("InitializationPlacement.class");
    let mut bytes = fs::read(&path).unwrap();
    let class = ClassFile::parse(&bytes).unwrap();
    let code = &class
        .methods
        .iter()
        .find(|method| method.name == "run")
        .unwrap()
        .code
        .as_ref()
        .unwrap()
        .bytes;
    let offset = bytes
        .windows(code.len())
        .position(|window| window == code)
        .unwrap();
    let instructions = bytecode::decode(code).unwrap();
    let allocation = instructions
        .iter()
        .position(|instruction| instruction.opcode == 0xbb)
        .unwrap();
    assert_eq!(instructions[allocation - 1].opcode, 0xb8);
    assert_eq!(instructions[allocation - 2].opcode, 0x15);
    assert_eq!(instructions[allocation + 1].opcode, 0x59);
    let start = instructions[allocation - 2].pc as usize;
    let new_pc = instructions[allocation].pc as usize;
    let end = instructions[allocation + 2].pc as usize;
    // Keep the preceding ternary intact, but make NEW initialize the target
    // before touch(). Select recovery merges this join into an earlier block;
    // trigger replay must still identify NEW in the original bytecode block.
    let reordered: Vec<_> = code[new_pc..end]
        .iter()
        .chain(&code[start..new_pc])
        .copied()
        .collect();
    bytes[offset + start..offset + end].copy_from_slice(&reordered);
    bytes[offset - 8..offset - 6].copy_from_slice(&3u16.to_be_bytes());
    bytes[6..8].copy_from_slice(&49u16.to_be_bytes());
    fs::write(path, bytes).unwrap();
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .args(["--no-api", "--threads", "1"]),
    );
    let emitted = fs::read_to_string(source.join("InitializationPlacement.java")).unwrap();
    assert!(!emitted.contains("Decompilation failed"), "{emitted}");
    assert!(
        emitted.contains("InitializationPlacementTarget.$sf$initialize$"),
        "{emitted}"
    );
    crate::common::compile(&crate::common::sources(&source), &rebuilt, None);
    let execute = |classes: &Path, branch: &str, mode: &str| {
        checked(
            Command::new(crate::common::java())
                .args(["-Xverify:all", "-cp"])
                .arg(classes)
                .arg("InitializationPlacement")
                .args([branch, mode]),
        )
        .stdout
    };
    for branch in ["left", "right"] {
        for mode in ["ok", "fail"] {
            assert_eq!(
                execute(&rebuilt, branch, mode),
                execute(&original, branch, mode),
                "{branch}, {mode}\n{emitted}"
            );
        }
    }
    assert_eq!(
        execute(&original, "left", "ok"),
        b"init;touch:7;constructor:7;\n"
    );
    assert_eq!(
        execute(&original, "right", "fail"),
        b"java.lang.ExceptionInInitializerError\ninit;\n"
    );
}

#[test]
fn original_new_boundaries_precede_branches_loops_and_alternative_constructors() {
    let fixture = Path::new("tests/fixtures/allocation_graph/classes");
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(fixture)
            .arg("AllocationGraphProbe"),
    )
    .stdout;
    assert_eq!(expected, b"10\n10\n3\n3\n101\n0\n");
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("source");
    let classes = work.path().join("classes");
    std::fs::create_dir(&classes).unwrap();
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(fixture)
            .arg(&source)
            .args(["--no-api", "--threads", "1"]),
    );
    let files: Vec<_> = std::fs::read_dir(&source)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "java")
        })
        .collect();
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&classes)
            .args(&files),
    );
    let actual = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&classes)
            .arg("AllocationGraphProbe"),
    )
    .stdout;
    assert_eq!(expected, actual);
    for (class, displaced) in [
        ("Branch", false),
        ("Failed", false),
        ("Loop", true),
        ("Choice", true),
    ] {
        let text =
            std::fs::read_to_string(source.join(format!("AllocationGraphProbe${class}.java")))
                .unwrap();
        assert_eq!(
            text.contains("public static void $sf$initialize$"),
            displaced,
            "only classes with displaced initialization effects require a trigger: {class}\n{text}"
        );
    }
    let text = std::fs::read_to_string(source.join("AllocationGraphProbe.java")).unwrap();
    for class in ["Branch", "Failed"] {
        let allocation = format!("new AllocationGraphProbe${class}(");
        let (_, arguments) = text
            .split_once(&allocation)
            .unwrap_or_else(|| panic!("missing direct allocation {allocation}:\n{text}"));
        // Either predicate orientation is readable. The conditional must stay
        // inside NEW so initialization still precedes the chosen argument.
        let selector = arguments
            .trim_start()
            .strip_prefix('!')
            .unwrap_or(arguments.trim_start());
        assert!(selector.starts_with("branchFlag ?"), "{text}");
    }
}
