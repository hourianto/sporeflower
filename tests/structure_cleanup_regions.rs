use crate::common::checked_stdout as checked;
use sporeflower::{
    api::{ApiCatalog, ApiOptions},
    classfile::ClassFile,
    naming::NamePlan,
    program::Program,
};
use std::{fs, process::Command};

#[test]
fn nested_cleanup_regions_preserve_pending_exits_and_exception_identity() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-g:none", "-d"])
            .arg(&original)
            .arg("tests/fixtures/java/NestedCleanup.java"),
    );
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("NestedCleanup"),
    );
    let classes =
        vec![ClassFile::parse(&fs::read(original.join("NestedCleanup.class")).unwrap()).unwrap()];
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
    let source = rebuilt.join("NestedCleanup.java");
    fs::write(&source, &output.source).unwrap();
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
            .arg("NestedCleanup"),
    );
    assert_eq!(actual, expected, "{}", output.source);
    let method = |name: &str| {
        output
            .source
            .split(&format!("public static int {name}("))
            .nth(1)
            .unwrap()
            .split("\n    public ")
            .next()
            .unwrap()
    };
    for (name, cleanups) in [
        ("nested", 2),
        ("shared", 1),
        ("handledInside", 1),
        ("conditionalCleanup", 1),
    ] {
        assert_eq!(
            method(name).matches("finally {").count(),
            cleanups,
            "{name}: {}",
            output.source
        );
    }
    // The monitor's early return skips cleanup; only the following step owns
    // an unconditional finally. Its exceptional monitor exit still retains
    // the original catch/rethrow, as the runtime trace above verifies.
    let conditional: String = method("conditionalCleanup").split_whitespace().collect();
    assert!(
        conditional.contains("try{NestedCleanup.step(1);return17;}finally{NestedCleanup.step(4);}"),
        "{}",
        output.source
    );
    assert_eq!(conditional.matches("NestedCleanup.step(4);").count(), 2);
    assert!(conditional.find("return13;").unwrap() < conditional.find("finally{").unwrap());
}

#[test]
fn nested_cleanup_certificates_forward_unknown_effects_and_preserve_pending_returns() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    let input = original.join("NestedUnknownCleanup.java");
    fs::write(
        &input,
        r#"
public class NestedUnknownCleanup {
    public static Class target;
    public static RuntimeException innerFailure, outerFailure;
    public static int trace;
    private static void invoke() {
        trace = trace * 10 + 1;
        try { target.newInstance(); }
        catch (InstantiationException caught) { throw new Error(caught); }
        catch (IllegalAccessException caught) { throw new Error(caught); }
    }
    public static int run(boolean early) {
        try {
            try {
                if (early) { invoke(); return 7; }
                invoke();
                return 9;
            } finally {
                trace = trace * 10 + 2;
                if (innerFailure != null) throw innerFailure;
            }
        } finally {
            trace = trace * 10 + 3;
            if (outerFailure != null) throw outerFailure;
        }
    }
}
"#,
    )
    .unwrap();
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-g:none", "-d"])
            .arg(&original)
            .arg(&input),
    );
    let classes = vec![
        ClassFile::parse(&fs::read(original.join("NestedUnknownCleanup.class")).unwrap()).unwrap(),
    ];
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    let run = program
        .resolve_method("NestedUnknownCleanup", "run", "(Z)I")
        .unwrap();
    assert!(program.method_throws(run).is_empty());
    assert!(program.throws_plan().unknown_escape(run));
    assert_eq!(program.throws_plan().diagnostic(run), None);
    for instruction in
        sporeflower::bytecode::decode(&program.method(run).code.as_ref().unwrap().bytes).unwrap()
    {
        if instruction.opcode != 0xbf {
            continue;
        }
        let fact = program
            .throws_plan()
            .throw_at_instruction(run, instruction.pc)
            .unwrap();
        if fact.caught {
            assert!(fact.unknown && fact.checked.is_empty() && !fact.requires_narrowing);
        }
    }
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let output =
        sporeflower::java::decompile_class(&program, &names, 0, &Default::default()).unwrap();
    assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
    assert!(
        !output.source.contains("throws Throwable"),
        "{}",
        output.source
    );
    let source = rebuilt.join("NestedUnknownCleanup.java");
    fs::write(&source, &output.source).unwrap();
    checked(
        Command::new(crate::common::javac())
            .args(["--release", "8", "-d"])
            .arg(&rebuilt)
            .arg(&source),
    );
    let driver = work.path().join("CleanupCertificateDriver.java");
    fs::write(
        &driver,
        r#"
public class CleanupCertificateDriver {
    public static Throwable payload;
    public static class Payload {
        public Payload() throws Throwable { if (payload != null) throw payload; }
    }
    public static void main(String[] args) {
        NestedUnknownCleanup.target = Payload.class;
        for (int failures = 0; failures < 8; failures++) {
            for (int branch = 0; branch < 2; branch++) {
                payload = (failures & 1) == 0 ? null : new java.io.IOException("body");
                NestedUnknownCleanup.innerFailure = (failures & 2) == 0 ? null : new RuntimeException("inner");
                NestedUnknownCleanup.outerFailure = (failures & 4) == 0 ? null : new RuntimeException("outer");
                NestedUnknownCleanup.trace = 0;
                int result = 0;
                Throwable caught = null;
                try { result = NestedUnknownCleanup.run(branch != 0); }
                catch (Throwable failure) { caught = failure; }
                Throwable expected = NestedUnknownCleanup.outerFailure != null ? NestedUnknownCleanup.outerFailure : NestedUnknownCleanup.innerFailure != null ? NestedUnknownCleanup.innerFailure : payload;
                if (caught != expected || NestedUnknownCleanup.trace != 123) throw new Error("cleanup identity/order");
                if (expected == null && result != (branch != 0 ? 7 : 9)) throw new Error("pending return");
            }
        }
        System.out.println("nested cleanup identity, order and pending returns preserved");
    }
}
"#,
    )
    .unwrap();
    for directory in [&original, &rebuilt] {
        checked(
            Command::new(crate::common::javac())
                .args(["--release", "8", "-cp"])
                .arg(directory)
                .arg("-d")
                .arg(directory)
                .arg(&driver),
        );
        assert_eq!(
            checked(
                Command::new(crate::common::java())
                    .arg("-cp")
                    .arg(directory)
                    .arg("CleanupCertificateDriver"),
            ),
            b"nested cleanup identity, order and pending returns preserved\n"
        );
    }
}
