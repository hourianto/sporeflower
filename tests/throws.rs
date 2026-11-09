use sporeflower::{
    api::{ApiCatalog, ApiOptions},
    bytecode,
    classfile::{ClassFile, Constant},
    program::{MemberId, Program},
};
use std::{fs, path::Path};

fn compile(directory: &Path, name: &str, source: &str) -> Vec<ClassFile> {
    let path = directory.join(format!("{name}.java"));
    fs::write(&path, source).unwrap();
    let result = std::process::Command::new(crate::common::javac())
        .args(["--release", "8", "-g:none", "-d"])
        .arg(directory)
        .arg(path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut paths: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "class")
        })
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| ClassFile::parse(&fs::read(path).unwrap()).unwrap())
        .collect()
}

fn stripped(classes: &mut [ClassFile]) {
    for class in classes {
        for method in &mut class.methods {
            method.exceptions.clear();
        }
    }
}

fn member(program: &Program<'_>, class: &str, method: &str, descriptor: &str) -> MemberId {
    program.resolve_method(class, method, descriptor).unwrap()
}

#[test]
fn infers_missing_constructor_and_transitive_call_declarations() {
    let dir = tempfile::tempdir().unwrap();
    let mut classes = compile(
        dir.path(),
        "CheckedFlow",
        r#"
import java.io.IOException;
public class CheckedFlow {
    public CheckedFlow() throws IOException { middle(); }
    static void leaf() throws IOException { throw new IOException("same object"); }
    static void middle() throws IOException { leaf(); }
    static void top() throws IOException { middle(); }
    static void caught() { try { top(); } catch (IOException ex) {} }
    static int library(java.io.InputStream in) throws IOException { return in.read(); }
    static void nullThrow() { throw null; }
    static void runtime() { throw new IllegalArgumentException(); }
}
"#,
    );
    stripped(&mut classes);
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    for name in ["<init>", "leaf", "middle", "top"] {
        let id = member(&program, "CheckedFlow", name, "()V");
        assert_eq!(program.method_throws(id), ["java/io/IOException"]);
        assert_eq!(program.throws_plan().diagnostic(id), None);
    }
    let top = member(&program, "CheckedFlow", "top", "()V");
    let trace = program.throws_plan().explain_effect(
        top,
        &sporeflower::throws::InferredEffect::Declared("java/io/IOException".into()),
    );
    assert_eq!(
        trace.first().unwrap().member,
        member(&program, "CheckedFlow", "leaf", "()V")
    );
    assert!(matches!(
        trace.first().unwrap().site,
        sporeflower::throws::EffectSite::Throw(_)
    ));
    assert_eq!(trace.last().unwrap().member, top);
    let id = member(
        &program,
        "CheckedFlow",
        "library",
        "(Ljava/io/InputStream;)I",
    );
    assert_eq!(program.method_throws(id), ["java/io/IOException"]);
    for name in ["caught", "nullThrow", "runtime"] {
        let id = member(&program, "CheckedFlow", name, "()V");
        assert!(
            program.method_throws(id).is_empty(),
            "{name}: {:?}",
            program.method_throws(id)
        );
        assert_eq!(program.throws_plan().diagnostic(id), None);
    }
}

#[test]
fn own_override_family_declarations_reach_interface_callers() {
    let dir = tempfile::tempdir().unwrap();
    let mut classes = compile(
        dir.path(),
        "Family",
        r#"
import java.io.IOException;
interface First { void run() throws IOException; }
interface Second { void run() throws IOException; }
class Worker implements First, Second { public void run() throws IOException { throw new IOException(); } }
public class Family {
    static void call(First first) throws IOException { first.run(); }
    static void other(Second second) throws IOException { second.run(); }
}
"#,
    );
    stripped(&mut classes);
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    for owner in ["First", "Second", "Worker"] {
        let id = member(&program, owner, "run", "()V");
        assert_eq!(program.method_throws(id), ["java/io/IOException"]);
        assert_eq!(program.throws_plan().diagnostic(id), None);
    }
    for (name, descriptor) in [("call", "(LFirst;)V"), ("other", "(LSecond;)V")] {
        assert_eq!(
            program.method_throws(member(&program, "Family", name, descriptor)),
            ["java/io/IOException"]
        );
    }
}

#[test]
fn reflection_exception_boundary_remains_explicitly_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let mut classes = compile(
        dir.path(),
        "Callback",
        r#"
public class Callback implements Runnable {
    public void run() { throw new RuntimeException(); }
}
"#,
    );
    // JVM execution ignores checked exception declarations. Replacing the NEW
    // type models an obfuscator-created callback Java source cannot declare.
    for class in &mut classes {
        for constant in &mut class.constant_pool.entries {
            if let Constant::Utf8(name) = constant
                && name == "java/lang/RuntimeException"
            {
                *name = "java/lang/InstantiationException".into();
            }
        }
    }
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    let id = member(&program, "Callback", "run", "()V");
    assert!(program.method_throws(id).is_empty());
    let diagnostic = program.throws_plan().diagnostic(id).unwrap();
    assert!(
        diagnostic.contains("java/lang/InstantiationException"),
        "{diagnostic}"
    );
    assert!(
        diagnostic.contains("java/lang/Runnable.run()V"),
        "{diagnostic}"
    );
    let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
    let output =
        sporeflower::java::decompile_class(&program, &names, id.class, &Default::default())
            .unwrap();
    assert_eq!(output.failed_methods, 1);
    assert!(
        output
            .diagnostics
            .iter()
            .any(|message| message.contains("external contract java/lang/Runnable.run()V"))
    );
}

#[test]
fn typed_rethrows_and_partial_catches_keep_the_source_checked_type() {
    let dir = tempfile::tempdir().unwrap();
    let mut classes = compile(
        dir.path(),
        "Rethrow",
        r#"
import java.io.IOException;
import java.io.EOFException;
public class Rethrow {
    static void leaf() throws IOException { throw new IOException(); }
    static void partial() throws IOException { try { leaf(); } catch (EOFException ex) {} }
    static void shared(boolean first) throws Throwable {
        Throwable saved;
        if (first) { try { leaf(); return; } catch (Throwable ex) { saved = ex; } }
        else { try { leaf(); return; } catch (Throwable ex) { saved = ex; } }
        throw saved;
    }
    static void narrow(IOException value) throws IOException { throw value; }
}
"#,
    );
    stripped(&mut classes);
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    assert_eq!(
        program.method_throws(member(&program, "Rethrow", "partial", "()V")),
        ["java/io/IOException"]
    );
    let id = member(&program, "Rethrow", "shared", "(Z)V");
    assert_eq!(program.method_throws(id), ["java/io/IOException"]);
    assert_eq!(program.throws_plan().diagnostic(id), None);
    let pc = bytecode::decode(&program.method(id).code.as_ref().unwrap().bytes)
        .unwrap()
        .into_iter()
        .find(|instruction| instruction.opcode == 0xbf)
        .unwrap()
        .pc;
    let fact = program.throws_plan().throw_fact(id, pc).unwrap();
    assert!(fact.caught && fact.requires_narrowing);
    assert_eq!(fact.checked, ["java/io/IOException"]);
    let id = member(&program, "Rethrow", "narrow", "(Ljava/io/IOException;)V");
    assert_eq!(program.method_throws(id), ["java/io/IOException"]);
    let method = program.method(id);
    let pc = bytecode::decode(&method.code.as_ref().unwrap().bytes)
        .unwrap()
        .iter()
        .find(|instruction| instruction.opcode == 0xbf)
        .unwrap()
        .pc;
    assert_eq!(
        program.throws_plan().throw_type(id, pc),
        Some("java/io/IOException")
    );
}

#[test]
fn catchall_of_unchecked_calls_does_not_invent_checked_throwable_effect() {
    let dir = tempfile::tempdir().unwrap();
    let mut classes = compile(
        dir.path(),
        "UncheckedRethrow",
        r#"
public class UncheckedRethrow implements Runnable {
    static void leaf() { throw new IllegalArgumentException(); }
    static void shared(boolean first) throws Throwable {
        Throwable saved;
        if (first) { try { leaf(); return; } catch (Throwable ex) { saved = ex; } }
        else { try { leaf(); return; } catch (Throwable ex) { saved = ex; } }
        throw saved;
    }
    public void run() { synchronized (this) { leaf(); } }
}
"#,
    );
    stripped(&mut classes);
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    for (name, descriptor) in [("shared", "(Z)V"), ("run", "()V")] {
        let id = member(&program, "UncheckedRethrow", name, descriptor);
        assert!(
            program.method_throws(id).is_empty(),
            "{name}: {:?}",
            program.method_throws(id)
        );
        assert_eq!(program.throws_plan().diagnostic(id), None);
        for instruction in
            bytecode::decode(&program.method(id).code.as_ref().unwrap().bytes).unwrap()
        {
            if instruction.opcode == 0xbf {
                let fact = program
                    .throws_plan()
                    .throw_fact(id, instruction.pc)
                    .unwrap();
                assert!(fact.caught && fact.requires_narrowing, "{fact:?}");
                assert!(fact.checked.is_empty());
            }
        }
    }
}

#[test]
fn recursive_call_cycles_reach_a_fixed_point() {
    let dir = tempfile::tempdir().unwrap();
    let mut classes = compile(
        dir.path(),
        "Recursive",
        r#"
import java.io.IOException;
public class Recursive {
    static void a(int n) throws IOException { if (n == 0) throw new IOException(); b(n-1); }
    static void b(int n) throws IOException { a(n); }
    static void c(int n) throws IOException { b(n); }
}
"#,
    );
    stripped(&mut classes);
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    for name in ["a", "b", "c"] {
        assert_eq!(
            program.method_throws(member(&program, "Recursive", name, "(I)V")),
            ["java/io/IOException"]
        );
    }
}

#[test]
fn missing_dependency_cannot_prove_a_catchall_rethrow_unchecked() {
    let dir = tempfile::tempdir().unwrap();
    let mut classes = compile(
        dir.path(),
        "UnknownBoundary",
        r#"
class MissingApi { static void invoke() {} }
public class UnknownBoundary {
    static void passThrough() { MissingApi.invoke(); }
    static void rethrow() throws Throwable {
        try { passThrough(); } catch (Throwable saved) { throw saved; }
    }
}
"#,
    );
    classes.retain(|class| class.name != "MissingApi");
    stripped(&mut classes);
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    assert!(
        program
            .method_throws(member(&program, "UnknownBoundary", "passThrough", "()V"))
            .is_empty()
    );
    let id = member(&program, "UnknownBoundary", "rethrow", "()V");
    assert_eq!(program.method_throws(id), ["java/lang/Throwable"]);
    let pc = bytecode::decode(&program.method(id).code.as_ref().unwrap().bytes)
        .unwrap()
        .into_iter()
        .find(|instruction| instruction.opcode == 0xbf)
        .unwrap()
        .pc;
    assert!(
        !program
            .throws_plan()
            .throw_fact(id, pc)
            .unwrap()
            .requires_narrowing
    );
}

#[test]
fn normalized_jsr_blocks_keep_their_own_rethrow_fact() {
    let dir = tempfile::tempdir().unwrap();
    let mut classes = compile(
        dir.path(),
        "JsrCatch",
        "public class JsrCatch { static void leaf() {} static void run() { leaf(); } }",
    );
    let class = &mut classes[0];
    class.major_version = 45;
    class.minor_version = 3;
    let method = class
        .methods
        .iter_mut()
        .find(|method| method.name == "run")
        .unwrap();
    let code = method.code.as_mut().unwrap();
    let invoke = bytecode::decode(&code.bytes)
        .unwrap()
        .into_iter()
        .find(|instruction| instruction.opcode == 0xb8)
        .unwrap();
    let bytecode::Operand::Constant(index) = invoke.operand else {
        panic!("expected method reference")
    };
    code.bytes = vec![
        0xa8,
        0,
        6,
        0xb1,
        0,
        0,
        0x4b,
        0xb8,
        (index >> 8) as u8,
        index as u8,
        0xa9,
        0,
        0x4c,
        0x2b,
        0xbf,
    ];
    code.max_stack = 1;
    code.max_locals = 2;
    code.exception_table = vec![sporeflower::classfile::ExceptionHandler {
        start_pc: 7,
        end_pc: 10,
        handler_pc: 12,
        catch_type: None,
    }];
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    let id = member(&program, "JsrCatch", "run", "()V");
    assert!(program.method_throws(id).is_empty());
    assert_eq!(program.throws_plan().diagnostic(id), None);
    let ir =
        sporeflower::ir::decompile_method(program.class(id.class), program.method(id)).unwrap();
    let mut seen = false;
    for block in &ir.blocks {
        if let sporeflower::ir::Terminator::Throw { pc, .. } = block.terminator {
            seen = true;
            let fact = program
                .throws_plan()
                .throw_at_instruction(id, pc)
                .expect("normalized block must retain rethrow fact");
            assert!(fact.requires_narrowing);
            assert!(fact.checked.is_empty());
        }
    }
    assert!(seen);
    assert!(
        program
            .throws_plan()
            .throw_fact(id, 14)
            .unwrap()
            .requires_narrowing
    );
    let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
    let source =
        sporeflower::java::decompile_class(&program, &names, id.class, &Default::default())
            .unwrap();
    assert_eq!(source.failed_methods, 0, "{:?}", source.diagnostics);
}

#[test]
fn narrowed_rethrow_source_preserves_checked_and_unchecked_object_identity() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original");
    let rebuilt = dir.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    let mut classes = compile(
        &original,
        "RethrowIdentity",
        r#"
public class RethrowIdentity {
    static int route;
    static void note(int value) { route = value; }
    static void leaf(Throwable value) throws java.io.IOException {
        if (value instanceof java.io.IOException) throw (java.io.IOException)value;
        if (value instanceof Error) throw (Error)value;
        throw (RuntimeException)value;
    }
    public static void bounce(Throwable value, boolean first) throws Throwable {
        Throwable saved;
        if (first) { try { leaf(value); return; } catch (Throwable ex) { saved = ex; } }
        else { try { leaf(value); return; } catch (Throwable ex) { saved = ex; } }
        note(first ? 17 : 23);
        throw saved;
    }
}
"#,
    );
    stripped(&mut classes);
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
    let output =
        sporeflower::java::decompile_class(&program, &names, 0, &Default::default()).unwrap();
    assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
    fs::write(rebuilt.join("RethrowIdentity.java"), &output.source).unwrap();
    // Compile the narrowed rethrow with the actual J2ME-era source rules when
    // that compiler is installed. A diamond folded into the ATHROW block used
    // to lose its throw fact and emit an undeclared, raw `throw Throwable`.
    let compiled = if let Some(legacy) = crate::common::legacy_javac() {
        let api_jar = dir.path().join("api.jar");
        api.write_jar(&api_jar).unwrap();
        crate::common::legacy_command(legacy, &api_jar)
            .arg("-d")
            .arg(&rebuilt)
            .arg(rebuilt.join("RethrowIdentity.java"))
            .output()
            .unwrap()
    } else {
        std::process::Command::new(crate::common::javac())
            .args(["--release", "8", "-d"])
            .arg(&rebuilt)
            .arg(rebuilt.join("RethrowIdentity.java"))
            .output()
            .unwrap()
    };
    assert!(
        compiled.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&compiled.stderr),
        output.source
    );
    let driver = r#"
public class IdentityDriver {
    public static void main(String[] args) {
        Throwable[] values = {new RuntimeException("runtime"), new Error("error"), new java.io.IOException("checked")};
        for (int i=0; i<values.length; i++) for (int route=0; route<2; route++) {
            try { RethrowIdentity.bounce(values[i], route == 0); throw new Error("returned"); }
            catch (Throwable result) { if (result != values[i]) throw new Error("changed identity"); }
            if (RethrowIdentity.route != (route == 0 ? 17 : 23)) throw new Error("changed selected argument");
        }
        System.out.println("same objects");
    }
}
"#;
    for directory in [&original, &rebuilt] {
        fs::write(directory.join("IdentityDriver.java"), driver).unwrap();
        let result = std::process::Command::new(crate::common::javac())
            .args(["--release", "8", "-classpath"])
            .arg(directory)
            .arg(directory.join("IdentityDriver.java"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stderr),
            output.source
        );
        let result = std::process::Command::new(crate::common::java())
            .arg("-cp")
            .arg(directory)
            .arg("IdentityDriver")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, b"same objects\n");
    }
}

#[test]
fn callback_boundary_hides_only_source_declaration_and_preserves_runtime_effects() {
    let dir = tempfile::tempdir().unwrap();
    let mut classes = compile(
        dir.path(),
        "CallbackFlow",
        r#"
class Dangerous implements Runnable { public void run() { throw new RuntimeException(); } }
class Harmless implements Runnable { public void run() {} }
public class CallbackFlow {
    static void direct(Dangerous callback) throws Throwable {
        try { callback.run(); } catch (Throwable saved) { throw saved; }
    }
    static void harmless(Harmless callback) throws Throwable {
        try { callback.run(); } catch (Throwable saved) { throw saved; }
    }
    static void dynamic(Runnable callback) throws Throwable {
        try { callback.run(); } catch (Throwable saved) { throw saved; }
    }
    static void pass(Dangerous callback) { callback.run(); }
    static void throughPass(Dangerous callback) throws Throwable {
        try { pass(callback); } catch (Throwable saved) { throw saved; }
    }
}
"#,
    );
    stripped(&mut classes);
    for class in &mut classes {
        if class.name != "Dangerous" {
            continue;
        }
        for constant in &mut class.constant_pool.entries {
            if let Constant::Utf8(name) = constant
                && name == "java/lang/RuntimeException"
            {
                *name = "java/io/IOException".into();
            }
        }
    }
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    let dangerous = member(&program, "Dangerous", "run", "()V");
    assert!(program.method_throws(dangerous).is_empty());
    assert_eq!(program.method_escapes(dangerous), ["java/io/IOException"]);
    assert_eq!(
        program.throws_plan().boundary(dangerous),
        ["java/io/IOException"]
    );
    assert_eq!(program.throws_plan().diagnostic(dangerous), None);
    let harmless = member(&program, "Harmless", "run", "()V");
    assert!(program.method_escapes(harmless).is_empty());
    assert!(program.throws_plan().boundary(harmless).is_empty());
    let via_pass = member(&program, "CallbackFlow", "pass", "(LDangerous;)V");
    assert!(program.method_throws(via_pass).is_empty());
    assert_eq!(program.method_escapes(via_pass), ["java/io/IOException"]);
    for (name, descriptor) in [
        ("direct", "(LDangerous;)V"),
        ("dynamic", "(Ljava/lang/Runnable;)V"),
        ("throughPass", "(LDangerous;)V"),
    ] {
        let id = member(&program, "CallbackFlow", name, descriptor);
        assert_eq!(program.method_throws(id), ["java/io/IOException"], "{name}");
    }
    let id = member(&program, "CallbackFlow", "harmless", "(LHarmless;)V");
    assert!(program.method_throws(id).is_empty());
}

fn replace_class_utf8(bytes: &[u8], old: &str, new: &str) -> Vec<u8> {
    let count = u16::from_be_bytes([bytes[8], bytes[9]]) as usize;
    let mut result = bytes[..10].to_vec();
    let mut offset = 10;
    let mut index = 1;
    while index < count {
        let tag = bytes[offset];
        offset += 1;
        result.push(tag);
        if tag == 1 {
            let size = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]) as usize;
            offset += 2;
            let value = &bytes[offset..offset + size];
            offset += size;
            let value = if value == old.as_bytes() {
                new.as_bytes()
            } else {
                value
            };
            result.extend((value.len() as u16).to_be_bytes());
            result.extend(value);
        } else {
            let size = match tag {
                3 | 4 | 9 | 10 | 11 | 12 | 17 | 18 => 4,
                5 | 6 => {
                    index += 1;
                    8
                }
                7 | 8 | 16 | 19 | 20 => 2,
                15 => 3,
                _ => panic!("unexpected constant-pool tag {tag}"),
            };
            result.extend(&bytes[offset..offset + size]);
            offset += size;
        }
        index += 1;
    }
    result.extend(&bytes[offset..]);
    result
}

#[test]
fn genuine_callback_escape_recompiles_without_changing_exception_identity() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original");
    let rebuilt = dir.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    compile(
        &original,
        "CallbackIdentity",
        r#"
interface OriginalContract { void run() throws java.io.IOException; }
public class CallbackIdentity implements OriginalContract {
    private final Throwable failure;
    public CallbackIdentity(Throwable failure) { this.failure = failure; }
    public void run() throws java.io.IOException {
        if (failure instanceof Error) throw (Error)failure;
        if (failure instanceof RuntimeException) throw (RuntimeException)failure;
        throw (java.io.IOException)failure;
    }
}
"#,
    );
    let class_path = original.join("CallbackIdentity.class");
    let bytes = replace_class_utf8(
        &fs::read(&class_path).unwrap(),
        "OriginalContract",
        "java/lang/Runnable",
    );
    fs::write(&class_path, &bytes).unwrap();
    let classes = [ClassFile::parse(&bytes).unwrap()];
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
    let output =
        sporeflower::java::decompile_class(&program, &names, 0, &Default::default()).unwrap();
    assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
    fs::write(rebuilt.join("CallbackIdentity.java"), &output.source).unwrap();
    let api_jar = dir.path().join("api.jar");
    api.write_jar(&api_jar).unwrap();
    let compiled = if let Some(legacy) = crate::common::legacy_javac() {
        crate::common::legacy_command(legacy, &api_jar)
            .arg("-d")
            .arg(&rebuilt)
            .arg(rebuilt.join("CallbackIdentity.java"))
            .output()
            .unwrap()
    } else {
        std::process::Command::new(crate::common::javac())
            .args(["--release", "8", "-d"])
            .arg(&rebuilt)
            .arg(rebuilt.join("CallbackIdentity.java"))
            .output()
            .unwrap()
    };
    assert!(
        compiled.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&compiled.stderr),
        output.source
    );
    let driver = r#"
public class CallbackDriver {
    static volatile boolean failed;
    static final class Narrow extends java.io.IOException {
        public String toString() { throw new Error("must not stringify payload"); }
    }
    static void check(Throwable value) {
        try { new CallbackIdentity(value).run(); failed = true; }
        catch (Throwable actual) { if (actual != value) failed = true; }
    }
    static void batch() {
        for (int i=0; i<100; i++) {
            check(new java.io.IOException()); check(new Narrow()); check(new RuntimeException()); check(new Error());
        }
    }
    public static void main(String[] args) throws Exception {
        Thread one = new Thread(new Runnable() { public void run() { batch(); } });
        Thread two = new Thread(new Runnable() { public void run() { batch(); } });
        one.start(); two.start(); one.join(); two.join(); batch();
        if (failed) throw new Error("changed exception identity");
        System.out.println("callback exception identity retained");
    }
}
"#;
    for directory in [&original, &rebuilt] {
        fs::write(directory.join("CallbackDriver.java"), driver).unwrap();
        let result = std::process::Command::new(crate::common::javac())
            .args(["--release", "8", "-sourcepath", "", "-classpath"])
            .arg(directory)
            .arg(directory.join("CallbackDriver.java"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let result = std::process::Command::new(crate::common::java())
            .arg("-cp")
            .arg(directory)
            .arg("CallbackDriver")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, b"callback exception identity retained\n");
    }
}

#[test]
fn reflective_constructor_effects_follow_class_value_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let mut classes = compile(
        dir.path(),
        "KnownReflection",
        r#"
public class KnownReflection {
    public static class Base { public Base() {} }
    public static final class Plain extends Base { public Plain() {} }
    public static final class Checked extends Base {
        public Checked() throws java.io.IOException { throw new java.io.IOException(); }
    }
    static void literal() throws Throwable {
        try { Checked.class.newInstance(); }
        catch (InstantiationException e) {} catch (IllegalAccessException e) {}
        catch (Throwable e) { throw e; }
    }
    static void named() throws Throwable {
        try { Class.forName("KnownReflection$Checked").newInstance(); }
        catch (InstantiationException e) {} catch (IllegalAccessException e) {}
        catch (ClassNotFoundException e) {} catch (Throwable e) { throw e; }
    }
    static void merged(boolean choose) throws Throwable {
        Class type = choose ? Checked.class : Plain.class;
        try { type.newInstance(); }
        catch (InstantiationException e) {} catch (IllegalAccessException e) {}
        catch (Throwable e) { throw e; }
    }
    static void bounded(Base value) throws Throwable {
        try { value.getClass().newInstance(); }
        catch (InstantiationException e) {} catch (IllegalAccessException e) {}
        catch (Throwable e) { throw e; }
    }
    static void allocated() throws Throwable {
        try { new Plain().getClass().newInstance(); }
        catch (InstantiationException e) {} catch (IllegalAccessException e) {}
        catch (Throwable e) { throw e; }
    }
    static void unknown(Class type) throws Throwable {
        try { type.newInstance(); }
        catch (InstantiationException e) {} catch (IllegalAccessException e) {}
        catch (Throwable e) { throw e; }
    }
}
"#,
    );
    stripped(&mut classes);
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    for (name, descriptor) in [
        ("literal", "()V"),
        ("named", "()V"),
        ("merged", "(Z)V"),
        ("bounded", "(LKnownReflection$Base;)V"),
    ] {
        let id = member(&program, "KnownReflection", name, descriptor);
        assert_eq!(program.method_throws(id), ["java/io/IOException"], "{name}");
        assert!(!program.throws_plan().unknown_escape(id), "{name}");
    }
    let allocated = member(&program, "KnownReflection", "allocated", "()V");
    assert!(program.method_throws(allocated).is_empty());
    let unknown = member(
        &program,
        "KnownReflection",
        "unknown",
        "(Ljava/lang/Class;)V",
    );
    assert!(program.method_throws(unknown).is_empty());
    let throw_pc = bytecode::decode(&program.method(unknown).code.as_ref().unwrap().bytes)
        .unwrap()
        .into_iter()
        .find(|instruction| instruction.opcode == 0xbf)
        .unwrap()
        .pc;
    let fact = program.throws_plan().throw_fact(unknown, throw_pc).unwrap();
    assert!(fact.unknown && fact.bridge && fact.requires_narrowing);
    let named = member(&program, "KnownReflection", "named", "()V");
    let reflection = bytecode::decode(&program.method(named).code.as_ref().unwrap().bytes).unwrap()
        .into_iter().find(|instruction| instruction.opcode == 0xb6 && matches!(instruction.operand, bytecode::Operand::Constant(index) if program.class(named.class).constant_pool.member_ref(index).is_ok_and(|method| method.name == "newInstance"))).unwrap();
    let facts = program
        .throws_plan()
        .reflection_at(named, reflection.pc)
        .unwrap();
    assert!(!facts.unknown);
    assert!(facts.checked.contains(&"java/io/IOException".to_owned()));
}

#[test]
fn narrower_sibling_override_does_not_constrain_unrelated_stream_contract() {
    let dir = tempfile::tempdir().unwrap();
    let mut classes = compile(
        dir.path(),
        "StreamContracts",
        r#"
class Buffer extends java.io.ByteArrayOutputStream {
    public void write(int value) { super.write(value); }
}
class Sink extends java.io.OutputStream {
    public void write(int value) throws java.io.IOException { throw new java.io.IOException(); }
}
class Parent { void call() throws java.io.IOException { throw new java.io.IOException(); } }
class Narrower extends Parent { void call() {} }
public class StreamContracts {}
"#,
    );
    stripped(&mut classes);
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    let sink = member(&program, "Sink", "write", "(I)V");
    assert_eq!(program.method_throws(sink), ["java/io/IOException"]);
    assert!(program.throws_plan().boundary(sink).is_empty());
    assert_eq!(program.throws_plan().diagnostic(sink), None);
    let buffer = member(&program, "Buffer", "write", "(I)V");
    assert!(program.method_throws(buffer).is_empty());
    assert!(program.throws_plan().boundary(buffer).is_empty());
    let parent = member(&program, "Parent", "call", "()V");
    assert_eq!(program.method_throws(parent), ["java/io/IOException"]);
    let child = member(&program, "Narrower", "call", "()V");
    assert!(program.method_throws(child).is_empty());
}

#[test]
fn hidden_effects_respect_symbolic_receiver_and_special_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let mut classes = compile(
        dir.path(),
        "ExactHiddenCalls",
        r#"
class RuntimeParent implements Runnable { public void run() {} }
class RuntimeChild extends RuntimeParent {
    public void run() { throw new RuntimeException(); }
    void exact() { super.run(); }
}
class UnknownChild extends RuntimeParent {
    private Class type;
    public void run() {
        try { type.newInstance(); }
        catch (InstantiationException e) {} catch (IllegalAccessException e) {}
    }
}
class RuntimeSibling extends RuntimeParent {}
public class ExactHiddenCalls { static void sibling(RuntimeSibling value) { value.run(); } }
"#,
    );
    for class in &mut classes {
        if class.name != "RuntimeChild" {
            continue;
        }
        for constant in &mut class.constant_pool.entries {
            if let Constant::Utf8(name) = constant
                && name == "java/lang/RuntimeException"
            {
                *name = "java/io/IOException".into();
            }
        }
    }
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    let parent = member(&program, "RuntimeParent", "run", "()V");
    let plan = program.throws_plan();
    let virtual_parent = plan.call_effects(parent, "RuntimeParent", true);
    assert_eq!(virtual_parent.checked, ["java/io/IOException"]);
    assert!(virtual_parent.unknown);
    for (owner, virtual_dispatch) in [("RuntimeParent", false), ("RuntimeSibling", true)] {
        let effects = plan.call_effects(parent, owner, virtual_dispatch);
        assert!(effects.checked.is_empty());
        assert!(!effects.unknown);
    }
    let child = member(&program, "RuntimeChild", "run", "()V");
    let child_effects = plan.call_effects(child, "RuntimeChild", true);
    assert_eq!(child_effects.checked, ["java/io/IOException"]);
    assert!(!child_effects.unknown);
    let exact = member(&program, "RuntimeChild", "exact", "()V");
    assert!(program.method_escapes(exact).is_empty());
}

#[test]
fn implicit_monitor_cleanup_forwards_unknown_effect_without_source_throwable() {
    let dir = tempfile::tempdir().unwrap();
    let mut classes = compile(
        dir.path(),
        "ReflectiveMonitor",
        r#"
public class ReflectiveMonitor implements Runnable {
    private final Object monitor = new Object();
    private Class type;
    public void run() {
        synchronized (monitor) {
            try { type.newInstance(); }
            catch (InstantiationException e) {} catch (IllegalAccessException e) {}
        }
    }
    static void checkedLeaf() throws java.io.IOException { throw new java.io.IOException(); }
    static void checked(Object monitor) throws java.io.IOException {
        synchronized (monitor) { checkedLeaf(); }
    }
}
"#,
    );
    stripped(&mut classes);
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    let run = member(&program, "ReflectiveMonitor", "run", "()V");
    assert!(program.method_throws(run).is_empty());
    assert!(program.throws_plan().unknown_escape(run));
    assert_eq!(program.throws_plan().diagnostic(run), None);
    let checked = member(
        &program,
        "ReflectiveMonitor",
        "checked",
        "(Ljava/lang/Object;)V",
    );
    assert_eq!(program.method_throws(checked), ["java/io/IOException"]);
    let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
    let source =
        sporeflower::java::decompile_class(&program, &names, run.class, &Default::default())
            .unwrap();
    assert_eq!(source.failed_methods, 0, "{:?}", source.diagnostics);
    assert!(source.source.contains("synchronized"));
    assert!(!source.source.contains("throws Throwable"));
    fs::write(dir.path().join("ReflectiveMonitor.java"), &source.source).unwrap();
    let result = std::process::Command::new(crate::common::javac())
        .args(["--release", "8"])
        .arg(dir.path().join("ReflectiveMonitor.java"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stderr),
        source.source
    );
}

#[test]
fn shared_cleanup_handler_keeps_the_unowned_rethrow_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let mut classes = compile(
        dir.path(),
        "PartialCleanup",
        r#"
public class PartialCleanup {
    static Class type;
    static int closes;
    static void invoke() {
        try { type.newInstance(); }
        catch (InstantiationException ignored) {} catch (IllegalAccessException ignored) {}
    }
    static void cleanup() { closes++; }
    static void other() { closes += 2; }
    static void run(boolean skip) {
        if (skip) { invoke(); other(); }
        try { invoke(); } finally { cleanup(); }
    }
}
"#,
    );
    stripped(&mut classes);
    let class = classes
        .iter_mut()
        .find(|class| class.name == "PartialCleanup")
        .unwrap();
    let method = class
        .methods
        .iter_mut()
        .find(|method| method.name == "run")
        .unwrap();
    let code = method.code.as_mut().unwrap();
    let first_call = bytecode::decode(&code.bytes)
        .unwrap()
        .into_iter()
        .find(|instruction| instruction.opcode == 0xb8)
        .unwrap()
        .pc;
    let handler = code
        .exception_table
        .iter()
        .find(|handler| handler.catch_type.is_none())
        .unwrap()
        .handler_pc;
    // The same original handler now serves two lexical scopes. The first
    // scope skips cleanup on success; only the second can become a finally.
    code.exception_table
        .push(sporeflower::classfile::ExceptionHandler {
            start_pc: first_call,
            end_pc: first_call + 3,
            handler_pc: handler,
            catch_type: None,
        });
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    let run = member(&program, "PartialCleanup", "run", "(Z)V");
    assert!(program.method_throws(run).is_empty());
    assert_eq!(program.throws_plan().diagnostic(run), None);
    let throw_pc = bytecode::decode(&program.method(run).code.as_ref().unwrap().bytes)
        .unwrap()
        .into_iter()
        .find(|instruction| instruction.opcode == 0xbf)
        .unwrap()
        .pc;
    let fact = program
        .throws_plan()
        .throw_at_instruction(run, throw_pc)
        .unwrap();
    assert!(fact.checked.is_empty());
    assert!(fact.unknown && fact.bridge && fact.requires_narrowing);
    let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
    let output =
        sporeflower::java::decompile_class(&program, &names, run.class, &Default::default())
            .unwrap();
    assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
    assert!(
        !output.source.contains("throws Throwable"),
        "{}",
        output.source
    );
    assert!(
        output.source.contains("throw $sf$rethrow"),
        "{}",
        output.source
    );
    fs::write(dir.path().join("PartialCleanup.java"), &output.source).unwrap();
    let result = std::process::Command::new(crate::common::javac())
        .args(["--release", "8"])
        .arg(dir.path().join("PartialCleanup.java"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn filtered_unknown_rethrow_preserves_identity_at_a_callback_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original");
    let rebuilt = dir.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    let mut classes = compile(
        &original,
        "ResidualBridge",
        r#"
interface BridgeContract { void run() throws Throwable; }
class BridgePayload {
    public BridgePayload() throws Throwable { throw ResidualBridge.pending; }
}
public class ResidualBridge implements BridgeContract {
    static Throwable pending;
    static Class kind;
    static int seen;
    static void invoke() {
        try { kind.newInstance(); }
        catch (InstantiationException ex) { seen = 2; }
        catch (IllegalAccessException ex) { seen = 3; }
    }
    public void run() throws Throwable {
        Throwable saved;
        try { invoke(); return; } catch (Throwable ex) { saved = ex; }
        seen = 1;
        throw saved;
    }
}
"#,
    );
    let class_path = original.join("ResidualBridge.class");
    let bytes = replace_class_utf8(
        &fs::read(&class_path).unwrap(),
        "BridgeContract",
        "java/lang/Runnable",
    );
    fs::write(&class_path, &bytes).unwrap();
    let changed = ClassFile::parse(&bytes).unwrap();
    *classes
        .iter_mut()
        .find(|class| class.name == "ResidualBridge")
        .unwrap() = changed;
    classes.retain(|class| class.name != "BridgeContract");
    stripped(&mut classes);
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    let run = member(&program, "ResidualBridge", "run", "()V");
    assert!(program.method_throws(run).is_empty());
    assert!(program.throws_plan().unknown_escape(run));
    assert_eq!(program.throws_plan().diagnostic(run), None);
    let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
    let mut sources = Vec::new();
    for class in 0..classes.len() {
        let output =
            sporeflower::java::decompile_class(&program, &names, class, &Default::default())
                .unwrap();
        assert_eq!(
            output.failed_methods, 0,
            "{:?}\n{}",
            output.diagnostics, output.source
        );
        let path = rebuilt.join(format!("{}.java", names.class_name(class)));
        fs::write(&path, &output.source).unwrap();
        sources.push(path);
    }
    let api_jar = dir.path().join("api.jar");
    api.write_jar(&api_jar).unwrap();
    let compiled = if let Some(legacy) = crate::common::legacy_javac() {
        crate::common::legacy_command(legacy, &api_jar)
            .arg("-d")
            .arg(&rebuilt)
            .args(&sources)
            .output()
            .unwrap()
    } else {
        std::process::Command::new(crate::common::javac())
            .args(["--release", "8", "-d"])
            .arg(&rebuilt)
            .args(&sources)
            .output()
            .unwrap()
    };
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let driver = r#"
public class ResidualDriver {
    static class Narrow extends InstantiationException {}
    static void check(Throwable expected, int seen) throws Exception {
        ResidualBridge.pending = expected;
        ResidualBridge.kind = Class.forName("BridgePayload");
        ResidualBridge.seen = 0;
        Throwable actual = null;
        try { ((Runnable)new ResidualBridge()).run(); } catch (Throwable value) { actual = value; }
        if (seen == 1 ? actual != expected : actual != null) throw new Error("changed exception identity");
        if (ResidualBridge.seen != seen) throw new Error("changed catch selection");
    }
    public static void main(String[] args) throws Exception {
        for (int i=0; i<20; i++) {
            check(new java.io.IOException(), 1);
            check(new Exception(), 1);
            check(new Throwable(), 1);
            check(new RuntimeException(), 1);
            check(new Error(), 1);
            check(new InstantiationException(), 2);
            check(new Narrow(), 2);
            check(new IllegalAccessException(), 3);
        }
        System.out.println("residual identities retained");
    }
}
"#;
    for directory in [&original, &rebuilt] {
        fs::write(directory.join("ResidualDriver.java"), driver).unwrap();
        let result = std::process::Command::new(crate::common::javac())
            .args(["--release", "8", "-sourcepath", "", "-classpath"])
            .arg(directory)
            .arg(directory.join("ResidualDriver.java"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let result = std::process::Command::new(crate::common::java())
            .args(["-Xverify:all", "-cp"])
            .arg(directory)
            .arg("ResidualDriver")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, b"residual identities retained\n");
    }
}

#[test]
fn rethrowing_a_filtered_family_and_late_join_restore_unsafe_transport() {
    let dir = tempfile::tempdir().unwrap();
    let mut classes = compile(
        dir.path(),
        "ResidualJoins",
        r#"
class MissingCalls { static void invoke() {} static void checked() throws PartialHierarchy {} }
class NarrowInstantiation extends InstantiationException {}
class MissingHierarchy extends IllegalAccessException {}
class PartialHierarchy extends MissingHierarchy {}
interface UnboundedContract { void run() throws Throwable; }
class UnsafeCallback implements UnboundedContract {
    public void run() throws Throwable {
        try { MissingCalls.invoke(); } catch (Throwable ex) { throw ex; }
    }
}
public class ResidualJoins {
    static Class kind;
    static void safe() {
        try { kind.newInstance(); }
        catch (InstantiationException ex) {} catch (IllegalAccessException ex) {}
    }
    static void reintroduce() throws Throwable {
        try {
            try { kind.newInstance(); }
            catch (InstantiationException ex) { throw ex; }
            catch (IllegalAccessException ex) {}
        } catch (Throwable ex) { throw ex; }
    }
    static void narrowCatch() throws Throwable {
        try {
            try { kind.newInstance(); }
            catch (NarrowInstantiation ex) {} catch (IllegalAccessException ex) {}
        } catch (Throwable ex) { throw ex; }
    }
    static void join(boolean unsafe) throws Throwable {
        try { safe(); if (unsafe) cycleA(1); }
        catch (Throwable ex) { throw ex; }
    }
    static void narrowedCast() throws IllegalAccessException {
        try { MissingCalls.invoke(); } catch (Throwable ex) { throw (IllegalAccessException)ex; }
    }
    static void partialHierarchy() throws Throwable {
        try { MissingCalls.checked(); } catch (PartialHierarchy ex) { throw ex; }
    }
    static void cycleA(int n) { if (n == 0) MissingCalls.invoke(); else cycleB(n - 1); }
    static void cycleB(int n) { cycleA(n); }
}
"#,
    );
    classes.retain(|class| {
        !matches!(
            class.name.as_str(),
            "MissingCalls" | "UnboundedContract" | "MissingHierarchy"
        )
    });
    for class in &mut classes {
        for constant in &mut class.constant_pool.entries {
            if let Constant::Utf8(name) = constant
                && name == "UnboundedContract"
            {
                *name = "java/lang/Runnable".into();
            }
        }
        for interface in &mut class.interfaces {
            if interface == "UnboundedContract" {
                *interface = "java/lang/Runnable".into();
            }
        }
    }
    stripped(&mut classes);
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    let callback = member(&program, "UnsafeCallback", "run", "()V");
    assert!(
        program
            .throws_plan()
            .diagnostic(callback)
            .unwrap()
            .contains("external contract java/lang/Runnable.run()V")
    );
    assert!(
        program
            .method_throws(member(&program, "ResidualJoins", "safe", "()V"))
            .is_empty()
    );
    for (name, descriptor) in [
        ("reintroduce", "()V"),
        ("narrowCatch", "()V"),
        ("join", "(Z)V"),
    ] {
        let method = member(&program, "ResidualJoins", name, descriptor);
        assert_eq!(
            program.method_throws(method),
            ["java/lang/Throwable"],
            "{name}"
        );
        let throws: Vec<_> = bytecode::decode(&program.method(method).code.as_ref().unwrap().bytes)
            .unwrap()
            .into_iter()
            .filter(|instruction| instruction.opcode == 0xbf)
            .collect();
        let fact = program
            .throws_plan()
            .throw_at_instruction(method, throws.last().unwrap().pc)
            .unwrap();
        assert!(fact.unknown && !fact.bridge, "{name}: {fact:?}");
    }
    let join = member(&program, "ResidualJoins", "join", "(Z)V");
    let narrowed = member(&program, "ResidualJoins", "narrowedCast", "()V");
    assert_eq!(
        program.method_throws(narrowed),
        ["java/lang/IllegalAccessException"]
    );
    let narrow_trace = program.throws_plan().explain_effect(
        narrowed,
        &sporeflower::throws::InferredEffect::Declared("java/lang/IllegalAccessException".into()),
    );
    assert!(
        narrow_trace
            .iter()
            .any(|step| step.effect
                == sporeflower::throws::InferredEffect::MayIllegalAccessException)
    );
    assert!(
        !narrow_trace
            .iter()
            .any(|step| step.effect
                == sporeflower::throws::InferredEffect::MayInstantiationException)
    );
    let partial = member(&program, "ResidualJoins", "partialHierarchy", "()V");
    assert_eq!(program.method_throws(partial), ["PartialHierarchy"]);
    let pc = bytecode::decode(&program.method(partial).code.as_ref().unwrap().bytes)
        .unwrap()
        .into_iter()
        .find(|instruction| instruction.opcode == 0xbf)
        .unwrap()
        .pc;
    let partial_fact = program
        .throws_plan()
        .throw_at_instruction(partial, pc)
        .unwrap();
    assert!(partial_fact.unknown && !partial_fact.bridge);
    let trace = program.throws_plan().explain_effect(
        join,
        &sporeflower::throws::InferredEffect::Declared("java/lang/Throwable".into()),
    );
    assert_eq!(
        trace.first().unwrap().member,
        member(&program, "ResidualJoins", "cycleA", "(I)V")
    );
    assert!(trace.iter().any(|step| step.member == join
        && matches!(step.site, sporeflower::throws::EffectSite::Throw(_))));
    for workers in [1, 2, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        let mut parallel = Program::new(&classes, &api.classes).unwrap();
        parallel.prepare_throws(&pool);
        for effect in [
            sporeflower::throws::InferredEffect::Declared("java/lang/Throwable".into()),
            sporeflower::throws::InferredEffect::Unknown,
            sporeflower::throws::InferredEffect::MayInstantiationException,
            sporeflower::throws::InferredEffect::MayIllegalAccessException,
        ] {
            assert_eq!(
                parallel.throws_plan().explain_effect(join, &effect),
                program.throws_plan().explain_effect(join, &effect),
                "{workers} workers: {effect:?}"
            );
        }
    }
    let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
    let unsupported =
        sporeflower::java::decompile_class(&program, &names, callback.class, &Default::default())
            .unwrap();
    assert_eq!(unsupported.failed_methods, 1);
    let owner = program.id("ResidualJoins").unwrap();
    let output =
        sporeflower::java::decompile_class(&program, &names, owner, &Default::default()).unwrap();
    assert_eq!(
        output.failed_methods, 0,
        "{:?}\n{}",
        output.diagnostics, output.source
    );
    fs::write(dir.path().join("ResidualJoins.java"), &output.source).unwrap();
    let result = std::process::Command::new(crate::common::javac())
        .args(["--release", "8", "-classpath"])
        .arg(dir.path())
        .arg(dir.path().join("ResidualJoins.java"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stderr),
        output.source
    );
}

#[test]
fn unbounded_missing_call_stays_implicit_in_proven_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let mut classes = compile(
        dir.path(),
        "MissingCleanup",
        r#"
class MissingCleanupApi { static void invoke() {} }
public class MissingCleanup implements Runnable {
    static int closed;
    public void run() { synchronized (this) { MissingCleanupApi.invoke(); } }
    static void cleanup() { closed++; }
    static void finalized() { try { MissingCleanupApi.invoke(); } finally { cleanup(); } }
    static void checked() throws java.io.IOException {
        try { throw new java.io.IOException(); } finally { cleanup(); }
    }
}
"#,
    );
    classes.retain(|class| class.name != "MissingCleanupApi");
    stripped(&mut classes);
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    for name in ["run", "finalized"] {
        let method = member(&program, "MissingCleanup", name, "()V");
        assert!(program.method_throws(method).is_empty(), "{name}");
        assert!(program.throws_plan().unknown_escape(method));
        assert_eq!(program.throws_plan().diagnostic(method), None);
        for instruction in
            bytecode::decode(&program.method(method).code.as_ref().unwrap().bytes).unwrap()
        {
            if instruction.opcode == 0xbf {
                let fact = program
                    .throws_plan()
                    .throw_at_instruction(method, instruction.pc)
                    .unwrap();
                assert!(fact.unknown && !fact.bridge && fact.checked.is_empty());
            }
        }
    }
    assert_eq!(
        program.method_throws(member(&program, "MissingCleanup", "checked", "()V")),
        ["java/io/IOException"]
    );
    let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
    let output =
        sporeflower::java::decompile_class(&program, &names, 0, &Default::default()).unwrap();
    assert_eq!(
        output.failed_methods, 0,
        "{:?}\n{}",
        output.diagnostics, output.source
    );
    assert!(output.source.contains("synchronized") && output.source.contains("finally"));
    assert!(!output.source.contains("$CheckedThrow"));
    fs::write(dir.path().join("MissingCleanup.java"), &output.source).unwrap();
    let result = std::process::Command::new(crate::common::javac())
        .args(["--release", "8", "-classpath"])
        .arg(dir.path())
        .arg(dir.path().join("MissingCleanup.java"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stderr),
        output.source
    );
}
