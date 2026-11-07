use crate::common::checked_stdout as checked;
use sporeflower::{
    api::{ApiCatalog, ApiOptions},
    bytecode,
    classfile::ClassFile,
    naming::NamePlan,
    program::Program,
};
use std::{fs, process::Command};

#[test]
fn nonthrowing_control_gaps_keep_one_owned_exception_region() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-g:none", "-d"])
            .arg(&original)
            .arg("tests/fixtures/java/SparseProtection.java"),
    );
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("SparseProtection"),
    );
    let mut class =
        ClassFile::parse(&fs::read(original.join("SparseProtection.class")).unwrap()).unwrap();
    for method in class
        .methods
        .iter_mut()
        .filter(|method| matches!(method.name.as_str(), "run" | "copiedLoops" | "impossible"))
    {
        let code = method.code.as_mut().unwrap();
        let instructions = bytecode::decode(&code.bytes).unwrap();
        if method.name == "impossible" {
            let handler = code.exception_table[0].clone();
            code.exception_table = instructions
                .iter()
                .enumerate()
                .filter(|(_, instruction)| instruction.opcode == 0xbe)
                .map(
                    |(index, instruction)| sporeflower::classfile::ExceptionHandler {
                        start_pc: instruction.pc,
                        end_pc: instructions[index + 1].pc,
                        ..handler.clone()
                    },
                )
                .collect();
            assert_eq!(code.exception_table.len(), 2);
            continue;
        }
        let mut sparse = Vec::new();
        // Removing only branch/goto instructions from the exception table changes
        // no JVM effect: these instructions cannot throw. This is the sparse table
        // shape used by optimizers, independently of any particular corpus class.
        for handler in &code.exception_table {
            let mut start = handler.start_pc;
            for (index, instruction) in instructions.iter().enumerate() {
                if instruction.pc < handler.start_pc || instruction.pc >= handler.end_pc {
                    continue;
                }
                if instruction.is_conditional() || instruction.opcode == 0xa7 {
                    if start < instruction.pc {
                        sparse.push(sporeflower::classfile::ExceptionHandler {
                            start_pc: start,
                            end_pc: instruction.pc,
                            ..handler.clone()
                        });
                    }
                    start = instructions
                        .get(index + 1)
                        .map_or(code.bytes.len() as u32, |next| next.pc);
                }
            }
            if start < handler.end_pc {
                sparse.push(sporeflower::classfile::ExceptionHandler {
                    start_pc: start,
                    ..handler.clone()
                });
            }
        }
        assert!(sparse.len() >= 2);
        // The second catch has a distinct target but can never run once its type
        // duplicates the preceding catch. Source completion must not count it.
        for handler in &mut sparse {
            if handler.catch_type.as_deref() == Some("java/lang/RuntimeException") {
                handler.catch_type = Some("java/lang/IllegalArgumentException".into());
            }
        }
        code.exception_table = sparse;
    }
    let classes = vec![class];
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let output =
        sporeflower::java::decompile_class(&program, &names, 0, &Default::default()).unwrap();
    assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
    let run = output
        .source
        .split("int run(")
        .nth(1)
        .unwrap()
        .split("int copiedLoops(")
        .next()
        .unwrap();
    assert_eq!(
        run.matches("catch (IllegalArgumentException").count(),
        1,
        "{}",
        output.source
    );
    assert!(!output.source.contains("$dispatch"), "{}", output.source);
    assert_eq!(
        output.source.matches("SparseProtection.next()").count(),
        1,
        "shared catch loop must remain one original continuation: {}",
        output.source
    );
    let source = rebuilt.join("SparseProtection.java");
    fs::write(&source, output.source).unwrap();
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-d"])
            .arg(&rebuilt)
            .arg(&source),
    );
    let actual = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&rebuilt)
            .arg("SparseProtection"),
    );
    assert_eq!(actual, expected);
}
