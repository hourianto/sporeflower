use crate::common::checked_stdout as checked;
use sporeflower::{classfile::ClassFile, ir, structure};
use std::{fs, process::Command};

#[test]
fn returning_fallthrough_groups_keep_one_normal_suffix_and_ordered_effects() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-g:none", "-d"])
            .arg(&original)
            .arg("tests/fixtures/java/ReconvergingSwitches.java"),
    );
    let class =
        ClassFile::parse(&fs::read(original.join("ReconvergingSwitches.class")).unwrap()).unwrap();
    for name in ["run", "protectedRun"] {
        let method = class
            .methods
            .iter()
            .find(|method| method.name == name)
            .unwrap();
        let ir = ir::decompile_method(&class, method).unwrap();
        let prepared = structure::PreparedIr::new(ir);
        let tree = structure::structure(&prepared).unwrap();
        assert_eq!(tree.statistics.labeled_regions, 0, "{name}: {tree:#?}");
        assert_eq!(tree.statistics.dispatch_regions, 0, "{name}: {tree:#?}");
    }
    let expected = checked(
        Command::new(crate::common::java())
            .args(["-Xverify:all", "-cp"])
            .arg(&original)
            .arg("ReconvergingSwitches"),
    );
    let api = work.path().join("api.jar");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let java = source.join("ReconvergingSwitches.java");
    let rendered = fs::read_to_string(&java).unwrap();
    assert!(!rendered.contains("label"), "{rendered}");
    // Each method keeps its common conditional suffix outside the switch.
    assert_eq!(rendered.matches("mark(60)").count(), 2, "{rendered}");
    assert_eq!(rendered.matches("mark(70)").count(), 2, "{rendered}");
    let calculation = rendered
        .split(" sharedCalculation(")
        .nth(1)
        .unwrap()
        .split("\n    }")
        .next()
        .unwrap();
    assert_eq!(calculation.matches(" - 32").count(), 1, "{calculation}");
    assert_eq!(calculation.matches("return ").count(), 1, "{calculation}");
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .arg(&java),
    );
    assert_eq!(
        expected,
        checked(
            Command::new(crate::common::java())
                .args(["-Xverify:all", "-cp"])
                .arg(&rebuilt)
                .arg("ReconvergingSwitches"),
        )
    );
    if let Some(legacy) = crate::common::legacy_javac() {
        let output = work.path().join("legacy");
        fs::create_dir_all(&output).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&output)
                .arg(&java),
        );
        assert_eq!(
            expected,
            checked(
                Command::new(crate::common::java())
                    .args(["-Xverify:all", "-cp"])
                    .arg(&output)
                    .arg("ReconvergingSwitches"),
            )
        );
    }
}
