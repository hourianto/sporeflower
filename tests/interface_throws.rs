use crate::common::checked_stdout as checked;
use std::{fs, process::Command};

#[test]
fn inherited_interface_source_contract_intersects_throws_without_losing_runtime_effects() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let rebuilt = work.path().join("rebuilt");
    let source = work.path().join("source");
    fs::create_dir(&original).unwrap();
    fs::create_dir(&rebuilt).unwrap();
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-g:none", "-d"])
            .arg(&original)
            .arg("tests/fixtures/java/InterfaceThrows.java"),
    );
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("InterfaceThrows"),
    );
    assert_eq!(expected, b"true\n");
    let api = work.path().join("api.jar");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let paths: Vec<_> = fs::read_dir(&source)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "java")
        })
        .collect();
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-d"])
            .arg(&rebuilt)
            .args(&paths),
    );
    assert_eq!(
        expected,
        checked(
            Command::new(crate::common::java())
                .arg("-cp")
                .arg(&rebuilt)
                .arg("InterfaceThrows")
        )
    );
    // Exercise the actual source-1.3 compiler when the local corpus toolchain is
    // present; the fixture and normal test remain portable without that SDK.
    if let Some(legacy) = crate::common::legacy_javac() {
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&rebuilt)
                .args(&paths),
        );
        assert_eq!(
            expected,
            checked(
                Command::new(crate::common::java())
                    .arg("-cp")
                    .arg(&rebuilt)
                    .arg("InterfaceThrows")
            )
        );
    }
}
