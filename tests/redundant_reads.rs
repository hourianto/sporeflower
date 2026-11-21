use crate::common::{compile, decompile, execute, source, sources};
use std::{fs, path::PathBuf};

#[test]
fn discarded_reads_keep_first_access_effects_and_release_source_expressions() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let output = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from("tests/fixtures/java/DiscardedReads.java")],
        &original,
        None,
    );
    let expected = execute(&original, "DiscardedReads");
    decompile(&original, &output);
    let emitted = fs::read_to_string(output.join("DiscardedReads.java")).unwrap();
    let joined = source::method(&emitted, "joined");
    assert!(!joined.contains("StringBuffer"), "{joined}");
    assert_eq!(joined.matches("ReadHolder.VALUE").count(), 2, "{joined}");
    let array = source::method(&emitted, "array");
    assert!(array.contains("new String[] {"), "{array}");
    assert_eq!(array.matches("ReadHolder.VALUE").count(), 2, "{array}");
    let volatile = source::method(&emitted, "volatileReads");
    assert_eq!(
        volatile.matches("ReadHolder.VOLATILE").count(),
        2,
        "{volatile}"
    );
    compile(&sources(&output), &rebuilt, None);
    assert_eq!(expected, execute(&rebuilt, "DiscardedReads"));
}

#[test]
fn inspection_retains_original_decisions_reads_and_slot_bindings() {
    use crate::common::{checked, compile_command};
    use sporeflower::session::{Session, SessionOptions};
    use std::process::Command;

    let work = tempfile::tempdir().unwrap();
    for debug in [false, true] {
        let root = work.path().join(if debug { "debug" } else { "stripped" });
        let original = root.join("original");
        fs::create_dir_all(&original).unwrap();
        checked(
            compile_command()
                .arg(if debug { "-g" } else { "-g:none" })
                .arg("-d")
                .arg(&original)
                .arg("tests/fixtures/java/BytecodeInspection.java"),
        );
        let expected = execute(&original, "BytecodeInspection");
        let bytes = fs::read(original.join("BytecodeInspection.class")).unwrap();
        let mut session = Session::default();
        session
            .add_class("BytecodeInspection.class", &bytes)
            .unwrap();
        for preserve in [false, true] {
            let output = root.join(if preserve { "preserved" } else { "simplified" });
            let mut command = Command::new(env!("CARGO_BIN_EXE_sporeflower"));
            let summary_path = root.join("summary.json");
            command
                .arg(&original)
                .arg("-o")
                .arg(&output)
                .args(["-j", "1"])
                .arg("--json")
                .arg(&summary_path);
            if preserve {
                command.arg("--preserve-bytecode");
            }
            checked(&mut command);
            let summary: serde_json::Value =
                serde_json::from_slice(&fs::read(&summary_path).unwrap()).unwrap();
            assert_eq!(summary["preserve_bytecode"], preserve);
            for counter in ["folded_decisions", "redundant_field_reads"] {
                assert_eq!(
                    summary["simplifications"][counter].as_u64().unwrap() > 0,
                    !preserve,
                    "{summary}"
                );
            }
            let text = fs::read_to_string(output.join("BytecodeInspection.java")).unwrap();
            session
                .prepare(SessionOptions {
                    preserve_bytecode: preserve,
                    ..Default::default()
                })
                .unwrap();
            let class = session.decompile("BytecodeInspection").unwrap();
            assert_eq!(class.failed_methods, 0, "{:?}", class.diagnostics);
            assert_eq!(text, class.source);
            let reads = source::method(&text, "reads");
            assert_eq!(
                reads.matches("field").count(),
                if preserve { 2 } else { 1 },
                "{reads}"
            );
            let decision = source::method(&text, "decision");
            assert_eq!(decision.contains("30"), preserve, "{decision}");
            assert!(decision.contains("zero()"), "{decision}");
            let bindings = source::method(&text, "bindings");
            assert_eq!(bindings.contains("+ 9"), preserve, "{bindings}");
            let capacity = source::method(&text, "capacity");
            assert!(
                capacity.contains("new Vector(") && !capacity.contains("$sf$"),
                "{capacity}"
            );
            if preserve {
                let choice = source::method(&text, "choose");
                assert!(
                    choice.contains("new short[] {") && choice.contains('?'),
                    "{choice}"
                );
            }
            if debug && preserve {
                let increments = source::method(&text, "increments");
                assert!(!increments.contains("cursor2"), "{increments}");
                for name in ["next", "doubled", "unused"] {
                    assert!(bindings.contains(name), "{bindings}");
                }
            }
            let rebuilt = root.join(if preserve {
                "rebuilt-preserved"
            } else {
                "rebuilt-default"
            });
            compile(&sources(&output), &rebuilt, None);
            assert_eq!(execute(&rebuilt, "BytecodeInspection"), expected);
        }
    }
}
