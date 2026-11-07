use sporeflower::{
    api::{ApiCatalog, ApiOptions},
    classfile::ClassFile,
    naming::NamePlan,
    program::Program,
};
use std::{fs, path::Path, process::Command};

fn javac(directory: &Path, paths: &[std::path::PathBuf]) {
    let output = Command::new(crate::common::javac())
        .args(["--release", "8", "-g:none", "-d"])
        .arg(directory)
        .args(paths)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// Model legacy optimizer layout: the normal cleanup follows the handlers, and
// its IOException arm shares the return used by the outer null/empty guard.
// The ownership proof must leave the shared pure return outside cleanup;
// empty scheduled handler/transfer fragments do not affect its equivalence.
fn shared_cleanup_return(mut bytes: Vec<u8>) -> Vec<u8> {
    use sporeflower::bytecode::{self, Operand};
    let class = ClassFile::parse(&bytes).unwrap();
    let original = class
        .methods
        .iter()
        .find(|method| method.name == "load")
        .unwrap()
        .code
        .as_ref()
        .unwrap();
    let instructions = bytecode::decode(&original.bytes).unwrap();
    let body_end = instructions
        .iter()
        .find(|instruction| instruction.opcode == 0xb8)
        .unwrap()
        .pc as usize
        + 3;
    assert_eq!(body_end, 32);
    assert_eq!(original.bytes[27], 0x4c); // astore_1: stream
    let close = instructions
        .iter()
        .find_map(|instruction| {
            let Operand::Constant(index) = instruction.operand else {
                return None;
            };
            let member = class.constant_pool.member_ref(index).ok()?;
            (member.name == "close" && member.descriptor == "()V").then_some(index)
        })
        .unwrap()
        .to_be_bytes();
    let io = (1..class.constant_pool.entries.len() as u16)
        .find(|&index| class.constant_pool.class_name(index).ok() == Some("java/io/IOException"))
        .unwrap();
    let mut code = original.bytes[..body_end].to_vec();
    for instruction in &instructions {
        if instruction.pc as usize >= body_end {
            break;
        }
        if let Operand::Branch(_) = instruction.operand {
            let pc = instruction.pc as usize;
            code[pc + 1..pc + 3].copy_from_slice(&((63 - pc) as i16).to_be_bytes());
        }
    }
    code.extend([0xa7, 0, 25]); // success -> normal cleanup at 57
    code.extend([0x4d, 0x2b, 0xb6, close[0], close[1], 0xb1]); // IOException: close; return
    code.extend([0x4e, 0xb1]); // close IOException: return
    code.extend([0x3a, 4, 0x2b, 0xb6, close[0], close[1]]); // Throwable: save; close
    code.extend([0xa7, 0, 5, 0x3a, 5]); // close IOException: ignore
    code.extend([0x19, 4, 0xbf]); // throw original object
    code.extend([0x2b, 0xb6, close[0], close[1], 0xb1]); // normal close; return
    code.extend([0x4d, 0xb1]); // close IOException: shared outer return
    assert_eq!(code.len(), 64);
    let handlers = [
        [28, 32, 35, io],
        [36, 40, 41, io],
        [28, 32, 43, 0],
        [45, 49, 52, io],
        [57, 61, 62, io],
    ];
    let mut attribute = Vec::new();
    attribute.extend(original.max_stack.to_be_bytes());
    attribute.extend(original.max_locals.max(6).to_be_bytes());
    attribute.extend((code.len() as u32).to_be_bytes());
    attribute.extend(code);
    attribute.extend((handlers.len() as u16).to_be_bytes());
    for entry in handlers {
        for value in entry {
            attribute.extend(value.to_be_bytes());
        }
    }
    attribute.extend(0u16.to_be_bytes()); // no stale frame/debug attributes
    let positions: Vec<_> = bytes
        .windows(original.bytes.len())
        .enumerate()
        .filter_map(|(index, value)| (value == original.bytes).then_some(index))
        .collect();
    assert_eq!(positions.len(), 1);
    let start = positions[0] - 8;
    let size = u32::from_be_bytes(bytes[start - 4..start].try_into().unwrap()) as usize;
    bytes[start - 4..start].copy_from_slice(&(attribute.len() as u32).to_be_bytes());
    bytes.splice(start..start + size, attribute);
    bytes[4..8].copy_from_slice(&[0, 3, 0, 45]); // inferred verifier frames
    bytes
}

#[test]
fn empty_cleanup_fragments_preserve_unknown_effects_and_callback_contracts() {
    use sporeflower::structure::{PreparedIr, Stmt};
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original");
    let rebuilt = temp.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    javac(
        &original,
        &["tests/fixtures/java/CleanupFragments.java".into()],
    );
    let path = original.join("CleanupFragments.class");
    fs::write(&path, shared_cleanup_return(fs::read(&path).unwrap())).unwrap();
    let mut classes: Vec<_> = fs::read_dir(&original)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "class")
        })
        .map(|path| ClassFile::parse(&fs::read(path).unwrap()).unwrap())
        .collect();
    classes.sort_by(|a, b| a.name.cmp(&b.name));
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    let load = program
        .resolve_method("CleanupFragments", "load", "()V")
        .unwrap();
    let run = program
        .resolve_method("CleanupFragments", "run", "()V")
        .unwrap();
    for member in [load, run] {
        assert!(program.method_throws(member).is_empty());
        assert!(program.throws_plan().unknown_escape(member));
        assert_eq!(program.throws_plan().diagnostic(member), None);
    }
    let fact = program.throws_plan().throw_fact(load, 56).unwrap();
    assert!(fact.caught && fact.unknown && !fact.requires_narrowing);
    assert!(fact.checked.is_empty());
    let ir = PreparedIr::new(
        sporeflower::ir::decompile_method(program.class(load.class), program.method(load)).unwrap(),
    );
    let tree = sporeflower::structure::structure(&ir).unwrap();
    let protected = tree.body.iter().find_map(|statement| {
        let Stmt::If { then_body, .. } = statement else {
            return None;
        };
        then_body
            .iter()
            .find(|statement| matches!(statement, Stmt::TryFinally { .. }))
    });
    assert!(protected.is_some(), "{tree:#?}");
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let mut paths = Vec::new();
    for index in 0..classes.len() {
        let output =
            sporeflower::java::decompile_class(&program, &names, index, &Default::default())
                .unwrap();
        assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
        let path = rebuilt.join(format!("{}.java", names.class_name(index)));
        fs::write(&path, output.source).unwrap();
        paths.push(path);
    }
    javac(&rebuilt, &paths);
    let mut executions = vec![original, rebuilt];
    if let Some(legacy) = crate::common::legacy_javac() {
        let stubs = temp.path().join("api.jar");
        api.write_jar(&stubs).unwrap();
        let output = temp.path().join("legacy");
        fs::create_dir_all(&output).unwrap();
        let result = crate::common::legacy_command(legacy, &stubs)
            .arg("-d")
            .arg(&output)
            .args(&paths)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        executions.push(output);
    }
    let driver = r#"public class CleanupFragmentsDriver {
        public static void main(String[] args) throws Exception {
            CleanupFragments.type = Class.forName("CleanupFragments$Payload");
            Throwable[] failures = {null, new java.io.IOException(), new Exception(), new RuntimeException(), new Error()};
            Throwable[] closes = {null, new java.io.IOException(), new RuntimeException()};
            for (int active=0;active<3;active++) for (int i=0;i<failures.length;i++) for (int j=0;j<closes.length;j++) {
                CleanupFragments.data = active==0 ? null : new byte[active-1];
                CleanupFragments.failure=failures[i]; CleanupFragments.closeFailure=closes[j]; CleanupFragments.closes=0;
                Throwable found=null;
                try { new CleanupFragments().run(); } catch(Throwable value) { found=value; }
                Throwable expected=active<2 ? null : closes[j] instanceof RuntimeException ? closes[j] : failures[i] instanceof java.io.IOException ? null : failures[i];
                if(found!=expected || CleanupFragments.closes!=(active==2?1:0)) throw new Error("cleanup identity/count");
            }
            System.out.println("shared cleanup identity retained");
        }
    }"#;
    for directory in &executions {
        let path = directory.join("CleanupFragmentsDriver.java");
        fs::write(&path, driver).unwrap();
        let result = Command::new(crate::common::javac())
            .args(["--release", "8", "-classpath"])
            .arg(directory)
            .arg(path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let result = Command::new(crate::common::java())
            .arg("-cp")
            .arg(directory)
            .arg("CleanupFragmentsDriver")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, b"shared cleanup identity retained\n");
    }
}

#[test]
fn threaded_finally_returns_do_not_fall_through_a_distinct_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original");
    let rebuilt = temp.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    let source = original.join("FinallyContinuation.java");
    fs::write(
        &source,
        include_str!("fixtures/java/FinallyContinuation.java"),
    )
    .unwrap();
    javac(&original, &[source]);
    let class_path = original.join("FinallyContinuation.class");
    let mut bytes = fs::read(&class_path).unwrap();
    let parsed = ClassFile::parse(&bytes).unwrap();
    let original_code = &parsed
        .methods
        .iter()
        .find(|method| method.name == "run")
        .unwrap()
        .code
        .as_ref()
        .unwrap()
        .bytes;
    let mut threaded = original_code.clone();
    let mut changed = 0;
    for instruction in sporeflower::bytecode::decode(original_code).unwrap() {
        if instruction.opcode == 0xa7
            && let sporeflower::bytecode::Operand::Branch(target) = instruction.operand
            && original_code[target as usize] == 0xb1
        {
            // A common legacy optimizer threads a goto directly to RETURN.
            // Keep offsets/table ranges fixed, padding its unused operand bytes.
            threaded[instruction.pc as usize..instruction.pc as usize + 3]
                .copy_from_slice(&[0xb1, 0, 0]);
            changed += 1;
        }
    }
    assert!(changed >= 2);
    let positions: Vec<_> = bytes
        .windows(original_code.len())
        .enumerate()
        .filter_map(|(index, value)| (value == original_code).then_some(index))
        .collect();
    assert_eq!(positions.len(), 1);
    bytes[positions[0]..positions[0] + threaded.len()].copy_from_slice(&threaded);
    // This fixture uses only legacy constants/opcodes. Version45 verification
    // infers frames, so the javac StackMapTable cannot mask the transformed CFG.
    bytes[4..8].copy_from_slice(&[0, 3, 0, 45]);
    fs::write(&class_path, &bytes).unwrap();
    let classes = vec![ClassFile::parse(&bytes).unwrap()];
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let output =
        sporeflower::java::decompile_class(&program, &names, 0, &Default::default()).unwrap();
    assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
    let source = rebuilt.join("FinallyContinuation.java");
    fs::write(&source, &output.source).unwrap();
    javac(&rebuilt, &[source]);
    let driver = r#"public class FinallyContinuationDriver {
        public static void main(String[] ignored) {
            int[] modes = {0,1,2,5};
            for(int i=0;i<modes.length;i++) {
                FinallyContinuation.closes=0;
                try { FinallyContinuation.run(modes[i]); } catch(IllegalStateException expected) {}
                if(FinallyContinuation.closes!=1) throw new Error("duplicate cleanup");
            }
            System.out.println("one cleanup per continuation");
        }
    }"#;
    for directory in [&original, &rebuilt] {
        let path = directory.join("FinallyContinuationDriver.java");
        fs::write(&path, driver).unwrap();
        let output = Command::new(crate::common::javac())
            .args(["--release", "8", "-classpath"])
            .arg(directory)
            .arg(path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = Command::new(crate::common::java())
            .arg("-cp")
            .arg(directory)
            .arg("FinallyContinuationDriver")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"one cleanup per continuation\n");
    }
}

#[test]
fn recovered_finally_keeps_pending_result_exception_identity_and_cleanup_failures() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original");
    let rebuilt = temp.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    let source = r#"
public class FinallyFlow implements Runnable {
    public static Throwable payload;
    public static RuntimeException cleanupFailure;
    public static int cleanups;
    private Class type = payloadClass();
    private static Class payloadClass() {
        try { return Class.forName("FinallyFlow$Payload"); }
        catch (ClassNotFoundException e) { throw new Error(); }
    }
    public static class Payload { public Payload() throws Throwable { if (payload != null) throw payload; } }
    public void run() {
        try {
            try { type.newInstance(); }
            catch (InstantiationException e) { throw new RuntimeException(); }
            catch (IllegalAccessException e) { throw new RuntimeException(); }
        } finally { cleanup(); }
    }
    public static void literalRun() {
        try { Class.forName("FinallyFlow$Payload").newInstance(); }
        catch (InstantiationException e) { throw new RuntimeException(); }
        catch (IllegalAccessException e) { throw new RuntimeException(); }
        catch (ClassNotFoundException e) { throw new RuntimeException(); }
        finally { cleanup(); }
    }
    public static int value(int input) {
        int pending = input + 7;
        try { return pending; } finally { cleanup(); }
    }
    public static RuntimeException bodyFailure = new RuntimeException("body");
    public static int trace, openCount;
    public static int closeMask;
    private void reflect() {
        try { type.newInstance(); }
        catch (InstantiationException e) { throw new RuntimeException(); }
        catch (IllegalAccessException e) { throw new RuntimeException(); }
    }
    private void checkpoint(int stage, int current) { if (stage == current) reflect(); }
    public void snapshots(int stage) {
        java.io.ByteArrayInputStream raw = null;
        java.io.DataInputStream data = null;
        try {
            checkpoint(stage, 0);
            raw = new java.io.ByteArrayInputStream(new byte[]{1});
            checkpoint(stage, 1);
            data = new java.io.DataInputStream(raw);
            reflect();
        } finally { snapshotCleanup(data, raw); }
    }
    public void staleSnapshots(int stage) {
        java.io.ByteArrayInputStream raw = null;
        java.io.DataInputStream data = null;
        try {
            if (stage == 0) reflect();
            raw = new java.io.ByteArrayInputStream(new byte[]{1});
            if (stage == 1) reflect();
            data = new java.io.DataInputStream(raw);
            if (stage == 2) reflect();
        } finally { snapshotCleanup(data, raw); }
    }
    private static void snapshotCleanup(Object data, Object raw) {
        closeMask = (data == null ? 0 : 2) | (raw == null ? 0 : 1);
        cleanup();
    }
    private static void body(int mode) throws java.io.IOException {
        if ((mode & 3) == 1) throw new java.io.IOException();
        if ((mode & 3) == 2) throw bodyFailure;
    }
    private static void close(int mode) throws java.io.IOException {
        cleanups++; if ((mode & 4) != 0) throw new java.io.IOException();
    }
    public static int nestedCatch(int mode) {
        try { body(mode); return 1; }
        catch (java.io.IOException saved) { return 2; }
        finally { try { close(mode); } catch (java.io.IOException ignored) {} }
    }
    private static void catchStep(int mode) { if ((mode & 4) != 0) throw bodyFailure; trace++; }
    public static int catchThrows(int mode) {
        try { body(mode); return 1; }
        catch (java.io.IOException saved) { catchStep(mode); return 2; }
        finally { cleanup(); }
    }
    public static void distinctContinuation(int mode) {
        try { body(mode); }
        catch (java.io.IOException saved) {
            try { catchStep(mode); } finally { cleanup(); }
            return;
        }
        cleanup();
    }
    public static int loopCleanup(int input) {
        try { body(input); return input + 3; }
        catch (java.io.IOException saved) { return -1; }
        finally {
            for (int i=0; i<2; i++) {
                switch (i) { case 0: trace=trace*10+4; break; default: trace=trace*10+5; }
            }
        }
    }
    public static int redirects(int count) {
        try {
            if (count<0) throw bodyFailure;
            int sum=0;
            for (int i=0; i<count; i++) { if (i<2) continue; sum+=i; }
            return sum;
        } finally { cleanup(); }
    }
    private static void resourceClose() { cleanups++; openCount--; }
    public static void closeThenReopen(boolean early) throws Throwable {
        try {
            if (early) { resourceClose(); openCount++; return; }
        } catch (Throwable saved) { resourceClose(); throw saved; }
        resourceClose();
    }
    private static void cleanup() { cleanups++; if (cleanupFailure != null) throw cleanupFailure; }
}
"#;
    fs::write(original.join("FinallyFlow.java"), source).unwrap();
    javac(&original, &[original.join("FinallyFlow.java")]);
    let mut classes: Vec<_> = fs::read_dir(&original)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "class")
        })
        .map(|path| ClassFile::parse(&fs::read(path).unwrap()).unwrap())
        .collect();
    classes.sort_by(|a, b| a.name.cmp(&b.name));
    for class in &mut classes {
        for method in &mut class.methods {
            method.exceptions.clear();
        }
    }
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    let literal = program
        .resolve_method("FinallyFlow", "literalRun", "()V")
        .unwrap();
    assert!(
        program.method_throws(literal).is_empty(),
        "{:?}",
        program.method_throws(literal)
    );
    assert!(
        program
            .method_escapes(literal)
            .contains(&"java/lang/Throwable".to_owned())
    );
    let snapshots = program
        .resolve_method("FinallyFlow", "snapshots", "(I)V")
        .unwrap();
    assert!(
        program.method_throws(snapshots).is_empty(),
        "{:?}",
        program.method_throws(snapshots)
    );
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    for name in ["nestedCatch", "loopCleanup", "snapshots"] {
        let class = program.class(literal.class);
        let method = class
            .methods
            .iter()
            .find(|method| method.name == name)
            .unwrap();
        let ir = sporeflower::structure::PreparedIr::new(
            sporeflower::ir::decompile_method(class, method).unwrap(),
        );
        let tree = sporeflower::structure::structure(&ir).unwrap();
        assert!(
            format!("{:?}", tree.body).contains("TryFinally"),
            "{name}: {:?}",
            tree.body
        );
    }
    let mut paths = Vec::new();
    for (index, class) in classes.iter().enumerate() {
        let output =
            sporeflower::java::decompile_class(&program, &names, index, &Default::default())
                .unwrap();
        assert_eq!(
            output.failed_methods, 0,
            "{:?}\n{}",
            output.diagnostics, output.source
        );
        if class.name == "FinallyFlow" {
            assert!(output.source.contains("finally {"), "{}", output.source);
        }
        let path = rebuilt.join(format!("{}.java", names.class_name(index)));
        fs::write(&path, output.source).unwrap();
        paths.push(path);
    }
    javac(&rebuilt, &paths);
    if let Some(legacy) = crate::common::legacy_javac() {
        let stubs = temp.path().join("api.jar");
        api.write_jar(&stubs).unwrap();
        let legacy_output = temp.path().join("legacy");
        fs::create_dir_all(&legacy_output).unwrap();
        let result = crate::common::legacy_command(legacy, &stubs)
            .arg("-d")
            .arg(&legacy_output)
            .args(&paths)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let driver = r#"
public class FinallyDriver {
    public static void main(String[] ignored) {
        Throwable[] values = {null, new java.io.IOException("checked"), new RuntimeException("runtime"), new Error("error")};
        for (int i=0; i<values.length; i++) for (int fail=0; fail<2; fail++) for(int route=0;route<2;route++) {
            FinallyFlow.payload=values[i]; FinallyFlow.cleanups=0;
            FinallyFlow.cleanupFailure=fail==0 ? null : new RuntimeException("cleanup");
            Throwable expected=fail==0 ? values[i] : FinallyFlow.cleanupFailure;
            Throwable found=null;
            try { if(route==0) new FinallyFlow().run(); else FinallyFlow.literalRun(); } catch(Throwable x) { found=x; }
            if(found!=expected || FinallyFlow.cleanups!=1) throw new Error("exception identity or cleanup count");
        }
        FinallyFlow.cleanupFailure=null; FinallyFlow.cleanups=0;
        for(int stage=0;stage<3;stage++) for(int i=0;i<values.length;i++) for(int route=0;route<2;route++) {
            FinallyFlow.payload=values[i]; FinallyFlow.cleanups=0; FinallyFlow.closeMask=-1;
            Throwable caught=null;
            try { if(route==0) new FinallyFlow().snapshots(stage); else new FinallyFlow().staleSnapshots(stage); } catch(Throwable x) { caught=x; }
            int mask=values[i]==null ? 3 : stage==0 ? 0 : stage==1 ? 1 : 3;
            if(caught!=values[i] || FinallyFlow.cleanups!=1 || FinallyFlow.closeMask!=mask) throw new Error("snapshot identity");
        }
        FinallyFlow.cleanups=0;
        if(FinallyFlow.value(5)!=12 || FinallyFlow.cleanups!=1) throw new Error("pending return");
        for(int mode=0;mode<8;mode++) {
            FinallyFlow.cleanups=0;
            int result=-1; Throwable caught=null;
            try { result=FinallyFlow.nestedCatch(mode); } catch(Throwable x) { caught=x; }
            if(FinallyFlow.cleanups!=1) throw new Error("nested cleanup count");
            if((mode&3)==2 ? caught!=FinallyFlow.bodyFailure : caught!=null || result!=((mode&3)==1?2:1)) throw new Error("nested catch continuation");
            FinallyFlow.cleanups=0; result=-1; caught=null;
            try { result=FinallyFlow.catchThrows(mode); } catch(Throwable x) { caught=x; }
            boolean bodyFailed=(mode&3)==2 || (mode&3)==1 && (mode&4)!=0;
            if(FinallyFlow.cleanups!=1 || (bodyFailed ? caught!=FinallyFlow.bodyFailure : caught!=null || result!=((mode&3)==1?2:1))) throw new Error("shared cleanup scope");
            FinallyFlow.cleanups=0; caught=null;
            try { FinallyFlow.distinctContinuation(mode); } catch(Throwable x) { caught=x; }
            int expectedCloses=(mode&3)==2?0:1;
            if(FinallyFlow.cleanups!=expectedCloses || (bodyFailed ? caught!=FinallyFlow.bodyFailure : caught!=null)) throw new Error("return crossed a distinct cleanup continuation");
        }
        FinallyFlow.trace=0;
        if(FinallyFlow.loopCleanup(4)!=7 || FinallyFlow.trace!=45) throw new Error("cleanup loop exit");
        FinallyFlow.cleanups=0;
        if(FinallyFlow.redirects(5)!=9 || FinallyFlow.cleanups!=1) throw new Error("internal redirect cleanup");
        FinallyFlow.cleanups=0; FinallyFlow.openCount=1;
        try { FinallyFlow.closeThenReopen(true); } catch(Throwable x) { throw new Error("unexpected"); }
        if(FinallyFlow.cleanups!=1 || FinallyFlow.openCount!=1) throw new Error("success return received extra cleanup");
        System.out.println("finally behavior retained");
    }
}
"#;
    for directory in [&original, &rebuilt] {
        fs::write(directory.join("FinallyDriver.java"), driver).unwrap();
        let output = Command::new(crate::common::javac())
            .args(["--release", "8", "-cp"])
            .arg(directory)
            .arg(directory.join("FinallyDriver.java"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = Command::new(crate::common::java())
            .arg("-cp")
            .arg(directory)
            .arg("FinallyDriver")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"finally behavior retained\n");
    }
}
