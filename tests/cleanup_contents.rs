use crate::common::checked_stdout as checked;
use sporeflower::{
    api::{ApiCatalog, ApiOptions},
    classfile::ClassFile,
    naming::NamePlan,
    program::Program,
};
use std::{fs, process::Command};

#[test]
fn copied_cleanup_preserves_literal_contents_and_continuation_assignments() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-g:none", "-d"])
            .arg(&original)
            .arg("tests/fixtures/java/CleanupContents.java"),
    );
    let class_path = original.join("CleanupContents.class");
    let mut bytes = fs::read(&class_path).unwrap();
    let parsed = ClassFile::parse(&bytes).unwrap();
    for name in ["postCleanup", "insideCleanup"] {
        let method = parsed
            .methods
            .iter()
            .find(|method| method.name == name)
            .unwrap();
        let code = method.code.as_ref().unwrap();
        let decoded = sporeflower::bytecode::decode(&code.bytes).unwrap();
        let branch = decoded
            .iter()
            .find(|instruction| instruction.is_conditional())
            .unwrap();
        let sporeflower::bytecode::Operand::Branch(join) = branch.operand else {
            panic!()
        };
        let connector=decoded.iter().find(|instruction|instruction.opcode==0xa7 && matches!(instruction.operand,sporeflower::bytecode::Operand::Branch(target) if target==join)).unwrap();
        let target = if name == "postCleanup" {
            connector.pc
        } else {
            decoded.iter().find(|instruction| {
                instruction.pc < connector.pc && instruction.opcode==0xb8
                    && matches!(instruction.operand,sporeflower::bytecode::Operand::Constant(index) if parsed.constant_pool.member_ref(index).unwrap().name=="finish")
            }).unwrap().pc
        };
        let positions: Vec<_> = bytes
            .windows(code.bytes.len())
            .enumerate()
            .filter_map(|(index, value)| (value == code.bytes).then_some(index))
            .collect();
        assert_eq!(positions.len(), 1);
        let offset = positions[0] + branch.pc as usize + 1;
        bytes[offset..offset + 2]
            .copy_from_slice(&((target as i32 - branch.pc as i32) as i16).to_be_bytes());
    }
    for name in [
        "optimizedNull",
        "overwrittenNull",
        "unknownNull",
        "throwingGuard",
        "optimizedLoop",
        "emptyEarly",
    ] {
        let method = parsed
            .methods
            .iter()
            .find(|method| method.name == name)
            .unwrap();
        let code = method.code.as_ref().unwrap();
        let decoded = sporeflower::bytecode::decode(&code.bytes).unwrap();
        let (index, branch) = decoded
            .iter()
            .enumerate()
            .find(|(_, instruction)| instruction.opcode == 0xc6)
            .unwrap();
        // Model a removed cleanup test while retaining its exact continuation.
        // Only optimizedNull has a proved null input. The other three graphs
        // intentionally skip an observable cleanup and are negative proofs.
        let first = if name == "throwingGuard" {
            assert_eq!(decoded[index - 1].opcode, 0x32); // aaload
            assert_eq!(decoded[index - 2].opcode, 0x03); // iconst_0
            index - 3
        } else {
            index - 1
        };
        assert!(matches!(decoded[first].opcode, 0x19 | 0x2a..=0x2d));
        let offsets: Vec<_> = bytes
            .windows(code.bytes.len())
            .enumerate()
            .filter_map(|(at, data)| (data == code.bytes).then_some(at))
            .collect();
        assert_eq!(offsets.len(), 1);
        let at = offsets[0];
        bytes[at + decoded[first].pc as usize..at + branch.pc as usize].fill(0x00);
        bytes[at + branch.pc as usize] = 0xa7; // ifnull -> goto, same target
    }
    // These transformed inputs use legacy instructions. The positive outside
    // edge enters a post-cleanup connector; the negative edge enters cleanup
    // itself. Execute these exact version45 graphs, without stale stack maps.
    bytes[4..8].copy_from_slice(&[0, 3, 0, 45]);
    fs::write(&class_path, &bytes).unwrap();
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-Xverify:all")
            .arg("-cp")
            .arg(&original)
            .arg("CleanupContents"),
    );
    let classes = vec![ClassFile::parse(&bytes).unwrap()];
    for name in [
        "flag",
        "unopened",
        "postCleanup",
        "optionalCleanup",
        "optimizedNull",
        "optimizedLoop",
        "emptyEarly",
        "convergingCleanup",
    ] {
        let method = classes[0]
            .methods
            .iter()
            .find(|method| method.name == name)
            .unwrap();
        let ir = sporeflower::ir::decompile_method(&classes[0], method).unwrap();
        let prepared = sporeflower::structure::PreparedIr::new(ir);
        let tree = sporeflower::structure::structure(&prepared).unwrap();
        assert!(
            format!("{tree:?}").contains("TryFinally"),
            "{name}: {tree:#?}"
        );
        if name == "optionalCleanup" {
            assert!(!format!("{tree:?}").contains("Throw("), "{tree:#?}");
        }
    }
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let output =
        sporeflower::java::decompile_class(&program, &names, 0, &Default::default()).unwrap();
    assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
    assert!(!output.source.contains("$dispatch"), "{}", output.source);
    let source = rebuilt.join("CleanupContents.java");
    fs::write(&source, &output.source).unwrap();
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-d"])
            .arg(&rebuilt)
            .arg(&source),
    );
    assert_eq!(
        expected,
        checked(
            Command::new(crate::common::java())
                .arg("-Xverify:all")
                .arg("-cp")
                .arg(&rebuilt)
                .arg("CleanupContents")
        ),
        "{}",
        output.source
    );
    if let Some(compiler) = crate::common::legacy_javac() {
        let stubs = work.path().join("api.jar");
        api.write_jar(&stubs).unwrap();
        checked(
            crate::common::legacy_command(compiler, &stubs)
                .arg("-d")
                .arg(&rebuilt)
                .arg(&source),
        );
        assert_eq!(
            expected,
            checked(
                Command::new(crate::common::java())
                    .arg("-Xverify:all")
                    .arg("-cp")
                    .arg(&rebuilt)
                    .arg("CleanupContents")
            ),
            "{}",
            output.source
        );
    }
}
