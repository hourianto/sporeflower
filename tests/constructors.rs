use crate::common::{checked, compile, files};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[test]
fn final_assignment_after_this_delegation_keeps_argument_super_and_branch_effects() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from(
            "tests/fixtures/java/ConstructorFinalDelegation.java",
        )],
        &original,
        None,
    );
    let path = original.join("ConstructorFinalDelegation.class");
    let mut bytes = fs::read(&path).unwrap();
    fn u16_at(bytes: &[u8], at: usize) -> usize {
        u16::from_be_bytes([bytes[at], bytes[at + 1]]) as usize
    }
    let mut at = 10;
    let mut index = 1;
    while index < u16_at(&bytes, 8) {
        let tag = bytes[at];
        at += 1;
        at += match tag {
            1 => 2 + u16_at(&bytes, at),
            3 | 4 | 9 | 10 | 11 | 12 => 4,
            5 | 6 => {
                index += 1;
                8
            }
            7 | 8 | 16 => 2,
            15 => 3,
            _ => panic!("unexpected tag {tag}"),
        };
        index += 1;
    }
    at += 6;
    at += 2 + 2 * u16_at(&bytes, at);
    assert_eq!(u16_at(&bytes, at), 3);
    at += 2;
    for _ in 0..3 {
        let flags = u16_at(&bytes, at) as u16;
        bytes[at..at + 2].copy_from_slice(&(flags | 0x10).to_be_bytes());
        assert_eq!(u16_at(&bytes, at + 6), 0);
        at += 8;
    }
    fs::write(&path, bytes).unwrap();
    let expected = crate::common::execute(&original, "ConstructorFinalDelegation");
    crate::common::decompile(&original, &source);
    let emitted = fs::read_to_string(source.join("ConstructorFinalDelegation.java")).unwrap();
    assert!(emitted.contains("final int value;"), "{emitted}");
    assert!(emitted.contains("final int other;"), "{emitted}");
    assert!(
        emitted.contains("++]"),
        "delegated postfix result must retain its update: {emitted}"
    );
    assert!(
        emitted.contains("$preinit"),
        "effectful unused arguments must remain before super: {emitted}"
    );
    compile(&files(&source, "java"), &rebuilt, None);
    let actual = crate::common::execute(&rebuilt, "ConstructorFinalDelegation");
    assert_eq!(
        String::from_utf8_lossy(&actual),
        String::from_utf8_lossy(&expected),
        "{emitted}"
    );
    let class = sporeflower::classfile::ClassFile::parse(
        &fs::read(rebuilt.join("ConstructorFinalDelegation.class")).unwrap(),
    )
    .unwrap();
    assert_ne!(
        class
            .fields
            .iter()
            .find(|field| field.name == "value")
            .unwrap()
            .access_flags
            & 0x10,
        0
    );
}

#[test]
fn branching_and_effectful_constructor_preludes_roundtrip() {
    let root = Path::new("tests/fixtures/constructors");
    let work = tempfile::tempdir().unwrap();
    let support = work.path().join("support");
    let original = work.path().join("original");
    let output = work.path().join("source");
    let recompiled = work.path().join("recompiled");
    let support_sources = files(&root.join("support"), "java");
    compile(&support_sources, &support, None);
    fs::create_dir_all(original.join("pkg")).unwrap();
    for class in files(&root.join("classes"), "class") {
        fs::copy(
            &class,
            original.join("pkg").join(class.file_name().unwrap()),
        )
        .unwrap();
    }
    for class in files(&support, "class") {
        fs::copy(&class, original.join(class.strip_prefix(&support).unwrap())).unwrap();
    }
    let driver = root.join("ConstructorDriver.java");
    compile(std::slice::from_ref(&driver), &original, Some(&original));
    let expected = crate::common::execute(&original, "ConstructorDriver");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(root.join("classes"))
            .arg(&output)
            .args(["--no-api", "--threads", "1", "--classpath"])
            .arg(&support),
    );
    let mut source = files(&output, "java");
    source.extend(support_sources);
    source.push(driver);
    compile(&source, &recompiled, None);
    let actual = crate::common::execute(&recompiled, "ConstructorDriver");
    assert_eq!(
        String::from_utf8_lossy(&actual),
        String::from_utf8_lossy(&expected)
    );
    let saved =
        fs::read_to_string(output.join("pkg/TestConstructorSavedArgumentsBeforeSuper.java"))
            .unwrap();
    assert!(
        !saved.contains("$Preinit"),
        "pure saved arguments do not require a carrier:\n{saved}"
    );
}

fn render_class(class: sporeflower::classfile::ClassFile) -> sporeflower::java::DecompiledClass {
    let classes = [class];
    let program = sporeflower::program::Program::new(&classes, &[]).unwrap();
    let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
    sporeflower::java::decompile_class(&program, &names, 0, &Default::default()).unwrap()
}

#[test]
fn guarded_noarg_super_keeps_each_constructor_body() {
    use sporeflower::{
        classfile::{ClassFile, Constant},
        java,
        naming::NamePlan,
        program::Program,
    };

    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("GuardedSuper.java");
    fs::write(
        &source,
        "public class GuardedSuper {
        static volatile int marker;
        GuardedSuper(int value) { marker = 7; }
    }",
    )
    .unwrap();
    let driver = work.path().join("Driver.java");
    fs::write(
        &driver,
        "class Driver {
        public static void main(String[] args) {
            new GuardedSuper(1);
            System.out.println(GuardedSuper.marker);
            new GuardedSuper(0);
            System.out.println(GuardedSuper.marker);
            new GuardedSuper(2);
            System.out.println(GuardedSuper.marker);
        }
    }",
    )
    .unwrap();
    compile(std::slice::from_ref(&source), work.path(), None);
    let template =
        ClassFile::parse(&fs::read(work.path().join("GuardedSuper.class")).unwrap()).unwrap();
    let field = template
        .constant_pool
        .entries
        .iter()
        .position(|entry| matches!(entry, Constant::Fieldref { .. }))
        .unwrap() as u16;
    let [hi, lo] = field.to_be_bytes();
    let original = &template.methods[0].code.as_ref().unwrap().bytes;
    assert_eq!(&original[..2], &[0x2a, 0xb7]); // aload_0; invokespecial
    let two_arms = |mut guard: Vec<u8>| {
        guard.extend([0x99, 0, 8]); // if zero, take the marker = 7 arm
        guard.extend(&original[..4]);
        guard.push(0xb1);
        guard.extend(original);
        guard
    };
    // Two entry tests, three distinct super() sites. Only the final arm sets 9.
    let mut three_arms = vec![
        0x1b,
        0x99,
        0,
        13,
        0x1b,
        0x04,
        0xa0,
        0,
        (8 + original.len()) as u8,
    ];
    three_arms.extend(&original[..4]);
    three_arms.push(0xb1);
    three_arms.extend(original);
    let mut nine = original.clone();
    assert_eq!(&nine[4..6], &[0x10, 7]); // bipush 7
    nine[5] = 9;
    three_arms.extend(nine);
    for (bytes, expected) in [
        (two_arms(vec![0x1b]), Some(b"0\n7\n7\n")),
        (two_arms(vec![0x04]), Some(b"0\n0\n0\n")),
        (two_arms(vec![0xb2, hi, lo]), None), // volatile getstatic must stay before super
        (three_arms, Some(b"0\n7\n9\n")),
    ] {
        let mut class = template.clone();
        let code = class.methods[0].code.as_mut().unwrap();
        code.bytes = bytes;
        code.max_stack = 2;
        code.local_variables.clear();
        let classes = [class];
        let program = Program::new(&classes, &[]).unwrap();
        let names = NamePlan::build(&program, &Default::default()).unwrap();
        let output = java::decompile_class(
            &program,
            &names,
            0,
            &java::JavaOptions {
                preserve_bytecode: true,
                ..Default::default()
            },
        )
        .unwrap();
        let Some(expected) = expected else {
            assert_eq!(output.failed_methods, 1, "{:?}", output.diagnostics);
            assert!(
                output
                    .diagnostics
                    .iter()
                    .any(|detail| detail.contains("2 reachable delegation sites")),
                "{:?}",
                output.diagnostics
            );
            continue;
        };
        assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
        assert!(
            output.source.contains("if (") && output.source.contains("marker = 7"),
            "{}",
            output.source
        );
        fs::write(&source, output.source).unwrap();
        compile(&[source.clone(), driver.clone()], work.path(), None);
        assert_eq!(crate::common::execute(work.path(), "Driver"), expected);
    }
}

#[test]
fn local_values_cross_super_without_reordering_argument_effects() {
    let root = Path::new("tests/fixtures/constructors");
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let recompiled = work.path().join("recompiled");
    let support = root.join("support/pkg/TestConstructorPreludeEffects.java");
    compile(std::slice::from_ref(&support), &original, None);
    let bytes = fs::read(root.join("boundary/TestConstructorUnliftablePrelude.class")).unwrap();
    fs::write(
        original.join("pkg/TestConstructorUnliftablePrelude.class"),
        &bytes,
    )
    .unwrap();
    let driver = root.join("ConstructorLiveDriver.java");
    compile(std::slice::from_ref(&driver), &original, Some(&original));
    let expected = crate::common::execute(&original, "ConstructorLiveDriver");
    let mut class = sporeflower::classfile::ClassFile::parse(&bytes).unwrap();
    // The zero-argument member writes uninitialized this, an independently
    // asserted unsupported boundary below. Isolate the two expressible members.
    class
        .methods
        .retain(|method| method.name != "<init>" || method.descriptor != "()V");
    let rendered = render_class(class);
    assert_eq!(rendered.failed_methods, 0, "{:?}", rendered.diagnostics);
    let source = work.path().join("TestConstructorUnliftablePrelude.java");
    fs::write(&source, &rendered.source).unwrap();
    compile(&[source, support, driver], &recompiled, None);
    let actual = crate::common::execute(&recompiled, "ConstructorLiveDriver");
    assert_eq!(
        String::from_utf8_lossy(&actual),
        String::from_utf8_lossy(&expected)
    );
}

#[test]
fn uninitialized_receiver_write_is_not_silently_moved_after_super() {
    let bytes =
        fs::read("tests/fixtures/constructors/boundary/TestConstructorUnliftablePrelude.class")
            .unwrap();
    let rendered = render_class(sporeflower::classfile::ClassFile::parse(&bytes).unwrap());
    assert_eq!(rendered.failed_methods, 1, "{:?}", rendered.diagnostics);
    assert!(
        rendered
            .diagnostics
            .iter()
            .any(|message| message.contains("uninitialized this"))
    );
}

#[test]
fn saved_value_survives_a_postfix_super_argument() {
    use sporeflower::{classfile::ClassFile, java, naming::NamePlan, program::Program};

    let work = tempfile::tempdir().unwrap();
    let file = work.path().join("Saved.java");
    fs::write(
        &file,
        r#"
        class Base { int base; Base(int x) { base = x; } }
        public class Saved extends Base {
            int before, after;
            Saved(int x) { super(x); before = x; after = x; }
            public static void main(String[] args) {
                Saved s = new Saved(10);
                System.out.println(s.base + ":" + s.before + ":" + s.after);
            }
        }"#,
    )
    .unwrap();
    compile(&[file], work.path(), None);
    let read = |name: &str| {
        ClassFile::parse(&fs::read(work.path().join(format!("{name}.class"))).unwrap()).unwrap()
    };
    let mut child = read("Saved");
    let constructor = child
        .methods
        .iter_mut()
        .find(|m| m.name == "<init>")
        .unwrap();
    let code = constructor.code.as_mut().unwrap();
    // Save x+1, pass the old x to super(x++), then write both live locals.
    let [
        0x2a,
        0x1b,
        0xb7,
        a,
        b,
        0x2a,
        0x1b,
        0xb5,
        c,
        d,
        0x2a,
        0x1b,
        0xb5,
        e,
        f,
        0xb1,
    ] = code.bytes[..]
    else {
        panic!("unexpected constructor: {:?}", code.bytes)
    };
    code.bytes = vec![
        0x1b, 0x04, 0x60, 0x3d, 0x2a, 0x1b, 0x84, 1, 1, 0xb7, a, b, 0x2a, 0x1c, 0xb5, c, d, 0x2a,
        0x1b, 0xb5, e, f, 0xb1,
    ];
    code.max_locals = 3;
    let classes = [child, read("Base")];
    let program = Program::new(&classes, &[]).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    for preserve_bytecode in [false, true] {
        let options = java::JavaOptions {
            preserve_bytecode,
            ..Default::default()
        };
        let output = java::decompile_class(&program, &names, 0, &options).unwrap();
        assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
        let rebuilt = work.path().join(format!("rebuilt-{preserve_bytecode}"));
        fs::create_dir_all(&rebuilt).unwrap();
        let file = rebuilt.join("Saved.java");
        fs::write(&file, output.source).unwrap();
        fs::copy(work.path().join("Base.class"), rebuilt.join("Base.class")).unwrap();
        compile(&[file], &rebuilt, None);
        assert_eq!(crate::common::execute(&rebuilt, "Saved"), b"10:11:11\n");
    }
}

#[test]
fn receiver_stores_cross_only_compatible_constructor_chains() {
    use sporeflower::{classfile::ClassFile, java, naming::NamePlan, program::Program};

    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("Capture.java");
    fs::write(
        &source,
        r#"
        class Base {
            int base;
            static int callbacks;
            Base(int x) { this(x + 1, 0); }
            private Base(int x, int ignored) {
                int copy = x;
                if (copy < 0) base = -copy;
                else base = copy + 2;
            }
            Base() { callbacks++; inspect(); }
            void inspect() {}
        }
        class Capture extends Base {
            final int capture;
            int tag, extra;
            static int calls;
            static int value(int x) { calls++; return x; }
            Capture(int x) { super(value(x)); capture = x; }
            Capture(int x, boolean observe) { super(); capture = x; }
            Capture() { super(2); tag = 1; capture = 0; }
            Capture(byte x) { this(); tag = 1; }
            Capture(char x) { this((byte)0); extra = 3; }
            Capture(short x) { this(); tag = 2; }
            void inspect() { base = capture; }
        }
    "#,
    )
    .unwrap();
    compile(&[source], work.path(), None);
    let read = |name: &str| {
        ClassFile::parse(&fs::read(work.path().join(format!("{name}.class"))).unwrap()).unwrap()
    };
    let mut capture = read("Capture");
    for method in capture.methods.iter_mut().filter(|m| m.name == "<init>") {
        // Move the last five-byte field assignment before delegation.
        let code = method.code.as_mut().unwrap();
        let end = code.bytes.len() - 1;
        assert_eq!(code.bytes[end - 3], 0xb5);
        code.bytes[..end].rotate_right(5);
    }
    let classes = [capture, read("Base")];
    let program = Program::new(&classes, &[]).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let driver = work.path().join("Driver.java");
    fs::write(
        &driver,
        r#"
        class Driver {
            public static void main(String[] args) {
                Capture c = new Capture(7);
                System.out.println(c.base + ":" + c.capture);
                Capture sibling = new Capture('x');
                System.out.println(sibling.tag + ":" + sibling.extra);
                try { new Capture(7, true); }
                catch (Error failure) {
                    System.out.println(failure.getMessage().indexOf("Capture.<init>(IZ)V") >= 0);
                }
                System.out.println(Base.callbacks);
            }
        }"#,
    )
    .unwrap();
    for preserve_bytecode in [false, true] {
        let options = java::JavaOptions {
            preserve_bytecode,
            ..Default::default()
        };
        let output = java::decompile_class(&program, &names, 0, &options).unwrap();
        assert_eq!(output.failed_methods, 2, "{:?}", output.diagnostics);
        assert_eq!(
            output.inspection_fallback_methods, 0,
            "{:?}",
            output.diagnostics
        );
        assert!(
            output
                .diagnostics
                .iter()
                .any(|d| d.contains("Capture.<init>(S)V")),
            "the sibling overwrites the caller's tag: {:?}",
            output.diagnostics
        );
        assert!(
            output
                .diagnostics
                .iter()
                .any(|message| message.contains("Capture.<init>(IZ)V")
                    && message.contains("uninitialized this")),
            "{:?}",
            output.diagnostics
        );
        assert!(
            output
                .source
                .lines()
                .any(|line| line.contains("capture = ") && !line.contains(" = 0;")),
            "{}",
            output.source
        );
        assert!(!output.source.contains("$preinit"), "{}", output.source);
        let rebuilt = work.path().join(format!("rebuilt-{preserve_bytecode}"));
        fs::create_dir_all(&rebuilt).unwrap();
        let source = rebuilt.join("Capture.java");
        fs::write(&source, output.source).unwrap();
        fs::copy(work.path().join("Base.class"), rebuilt.join("Base.class")).unwrap();
        compile(&[source, driver.clone()], &rebuilt, None);
        assert_eq!(
            crate::common::execute(&rebuilt, "Driver"),
            b"10:7\n1:3\ntrue\n0\n"
        );
    }

    // A library stub can only establish the signature of the deployed parent.
    let (own, library) = classes.split_at(1);
    let program = Program::new(own, library).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let output = java::decompile_class(&program, &names, 0, &Default::default()).unwrap();
    assert_eq!(output.failed_methods, 6, "{:?}", output.diagnostics);
}

#[test]
fn nonstatic_member_rebuilds_with_the_same_enclosing_instance_and_constructor_abi() {
    use crate::common::{execute, legacy_compile, sources};
    use sporeflower::classfile::ClassFile;

    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("Owner.java");
    fs::write(
        &source,
        r#"
        public class Owner {
            int base;
            public static class Parent {
                int bias;
                Parent(int x) { int local = x; bias = local + 1; }
                Parent(boolean observe) { inspect(); }
                void inspect() {}
            }
            public class Part {
                final int n;
                public Part(int n) { this.n = n; }
                public int value() { return Owner.this.base + n; }
                public String label() { return "Part"; }
                public class Unit extends Parent {
                    public Unit() { super(Part.this.n); }
                    public int compare(long a, long b) { return (int) (a - b); }
                    public int value() { return Part.this.value() + bias; }
                    public java.util.Date date(int n) {
                        System.out.print("date ");
                        return new java.util.Date(Owner.argument(n));
                    }
                }
                public int sum(int count) {
                    int total = 0;
                    while (count-- > 0) total += value();
                    return total;
                }
                public long scale(long x) { return x / 10000000000L; }
            }
            Part make(int n) { return new Part(n); }
            static Part attach(Owner o, int n) { return o.new Part(n); }
            static int argument(int n) { System.out.print("arg "); return n + 1; }
            static Part evaluated(Owner owner, int n) { return owner.new Part(argument(n) * 2); }
            static int nested(Owner owner, int n) {
                try { return owner.new Part(argument(n)).value(); }
                catch (NullPointerException expected) { return -1; }
            }
            static Part late(Owner owner, int n) { return owner.new Part(argument(n) + argument(n)); }
            static Owner choose(int path) {
                System.out.print("choose ");
                if (path == 2) throw new IllegalArgumentException();
                return path == 0 ? new Owner() : null;
            }
            static Part local(int path) {
                Owner selected = choose(path);
                System.out.print("after ");
                return selected.new Part(12);
            }
            public static void main(String[] args) {
                Owner o = new Owner();
                o.base = 20;
                System.out.println(o.make(2).value());
                System.out.println(attach(o, 3).value());
                Part localPart = local(0);
                System.out.println(localPart.label());
                System.out.println(localPart.scale(20000000000L));
                System.out.println(evaluated(o, 3).sum(2));
                System.out.println(nested(o, 3));
                System.out.println(nested(null, 3));
                Part.Unit unit = localPart.new Unit();
                System.out.println(unit.compare(100L, 4L));
                System.out.println(unit.date(5).getTime() + unit.value());
                try { evaluated(null, 3); }
                catch (NullPointerException expected) { System.out.println("checked before arg"); }
                try { late(null, 3); }
                catch (NullPointerException expected) { System.out.println("late null"); }
                try { local(2); }
                catch (IllegalArgumentException expected) { System.out.println("threw"); }
                try { local(1); }
                catch (NullPointerException expected) { System.out.println("local null"); }
                try { attach(null, 4); }
                catch (NullPointerException expected) { System.out.println("null"); }
            }
        }
    "#,
    )
    .unwrap();
    compile(&[source], &original, None);
    // JVM lcmp can produce the returned integer directly. Java needs a
    // comparison helper, whose static declaration must live outside both inner classes.
    let part_path = original.join("Owner$Part$Unit.class");
    let mut bytes = fs::read(&part_path).unwrap();
    let at = bytes
        .windows(5)
        .position(|b| b == [0x1f, 0x21, 0x65, 0x88, 0xac])
        .unwrap();
    bytes[at + 2..at + 4].copy_from_slice(&[0x94, 0x00]);
    // NEW/DUP before the print requires an argument helper. It must be an
    // instance method on Unit, even though the extracted region never reads this.
    let unit = ClassFile::parse(&bytes).unwrap();
    let date = &unit
        .methods
        .iter()
        .find(|m| m.name == "date")
        .unwrap()
        .code
        .as_ref()
        .unwrap()
        .bytes;
    let at = bytes.windows(date.len()).position(|w| w == date).unwrap();
    let new = sporeflower::bytecode::decode(date)
        .unwrap()
        .into_iter()
        .find(|op| op.opcode == 0xbb)
        .unwrap()
        .pc as usize;
    bytes[at..at + new + 4].rotate_right(4);
    fs::write(part_path, bytes).unwrap();
    // Move a side effect before the explicit outer null check. Both operand
    // schedules are valid bytecode, but Java checks the qualifier first.
    let owner_path = original.join("Owner.class");
    let mut bytes = fs::read(&owner_path).unwrap();
    let owner = ClassFile::parse(&bytes).unwrap();
    let source = &owner
        .methods
        .iter()
        .find(|m| m.name == "late")
        .unwrap()
        .code
        .as_ref()
        .unwrap()
        .bytes;
    let at = bytes
        .windows(source.len())
        .position(|w| w == source)
        .unwrap();
    let alloc = &source[..4]; // new, dup
    let guard = &source[4..10]; // aload_0, dup, getClass/requireNonNull, pop
    let argument = &source[10..14]; // iload_1, invokestatic argument
    let init = &source[source.len() - 4..]; // invokespecial, areturn
    // Keep the argument on the operand stack across the call, with no
    // authored local to force the source scheduler's evaluation order.
    let mut changed = [alloc, &guard[..1], argument, &[0x5f], &guard[1..], &[0x5f]].concat();
    changed.resize(source.len() - init.len(), 0);
    changed.extend(init);
    bytes[at..at + source.len()].copy_from_slice(&changed);
    fs::write(owner_path, bytes).unwrap();
    let expected = execute(&original, "Owner");
    assert_eq!(
        expected,
        b"22\n23\nchoose after Part\n2\narg 56\narg 24\n-1\n1\ndate arg 31\nchecked before arg\narg late null\nchoose threw\nchoose after local null\nnull\n"
    );
    let read = |name: &str| ClassFile::parse(&fs::read(original.join(name)).unwrap()).unwrap();
    let owner = read("Owner.class");
    let part = read("Owner$Part.class");
    // A null enclosing argument is legal at a raw JVM call. Only javac's
    // checked source idiom can become qualified new. JSR normalization may
    // also assign a different meaning to the raw check's bytecode position.
    let root = |classes: &[ClassFile], id| {
        let program = sporeflower::program::Program::new(classes, &[]).unwrap();
        let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
        sporeflower::java::SourceUnits::new(&program, &names).root(id)
    };
    let grouped = |classes: &[ClassFile]| root(classes, 1) == 0;
    assert!(grouped(&[owner.clone(), part.clone()]));
    let mut parent = read("Owner$Parent.class");
    let mut family = [
        owner.clone(),
        part.clone(),
        read("Owner$Part$Unit.class"),
        parent.clone(),
    ];
    assert_eq!(root(&family, 2), 0);
    // The alternate overload invokes a virtual method on the uninitialized
    // child, which can distinguish pre- from post-super capture assignment.
    let observing = parent
        .methods
        .iter()
        .find(|m| m.descriptor == "(Z)V")
        .unwrap()
        .code
        .clone();
    parent
        .methods
        .iter_mut()
        .find(|m| m.descriptor == "(I)V")
        .unwrap()
        .code = observing;
    family[3] = parent;
    assert_eq!(root(&family, 2), 2);
    for subroutine in [false, true] {
        let mut changed = owner.clone();
        let caller = changed
            .methods
            .iter_mut()
            .find(|m| m.name == "attach")
            .unwrap();
        let code = &mut caller.code.as_mut().unwrap().bytes;
        if subroutine {
            code.extend([0xa8, 0, 3, 0x4d, 0xa9, 2]); // jsr, astore_2, ret 2
        } else {
            let guard = code
                .windows(2)
                .position(|pair| pair == [0x59, 0xb8] || pair == [0x59, 0xb6])
                .unwrap();
            code[guard..guard + 5].fill(0);
        }
        assert!(!grouped(&[changed, part.clone()]));
    }
    let mut replaced_this = owner.clone();
    replaced_this
        .methods
        .iter_mut()
        .find(|method| method.name == "make")
        .unwrap()
        .code
        .as_mut()
        .unwrap()
        .bytes
        .splice(..0, [0x01, 0x4b]); // aconst_null, astore_0
    assert!(!grouped(&[replaced_this, part.clone()]));
    if owner
        .constant_pool
        .entries
        .iter()
        .any(|c| matches!(c, sporeflower::classfile::Constant::Utf8(n) if n == "java/util/Objects"))
    {
        let mut shim = owner.clone();
        shim.name = "java/util/Objects".into();
        shim.fields.clear();
        shim.methods.clear();
        shim.inner_classes.clear();
        assert!(!grouped(&[owner, part, shim]));
    }
    for preserve in [false, true] {
        let output = work
            .path()
            .join(if preserve { "preserved" } else { "simplified" });
        let api = work.path().join("api.jar");
        let mut command = Command::new(env!("CARGO_BIN_EXE_sporeflower"));
        command.arg(&original).arg(&output).arg("--stubs").arg(&api);
        if preserve {
            command.arg("--preserve-bytecode");
        }
        checked(&mut command);
        let files = sources(&output);
        assert_eq!(files.len(), 1, "{files:?}");
        let java = fs::read_to_string(&files[0]).unwrap();
        let nested = crate::common::source::method(&java, "nested");
        assert!(
            nested.contains(".new Part(")
                && !nested.contains("getClass")
                && !nested.contains("requireNonNull"),
            "{nested}"
        );
        for part in [
            "public class Part {",
            "public class Unit extends ",
            "private long $sf$argument_",
            "Part.this.value()",
            "private static final class $sf$Support",
            "public Part(int ",
            "Owner.this.base",
            ".new Part(",
            "return new Part(",
        ] {
            assert!(java.contains(part), "{part}: {java}");
        }
        assert!(
            !java.contains("requireNonNull") && !java.contains("this$0"),
            "{java}"
        );
        let rebuilt = output.join("classes");
        compile(&files, &rebuilt, None);
        assert_eq!(execute(&rebuilt, "Owner"), expected);
        let legacy = output.join("legacy");
        if legacy_compile(&files, &legacy, &api) {
            let inner =
                ClassFile::parse(&fs::read(legacy.join("Owner$Part.class")).unwrap()).unwrap();
            assert!(
                inner
                    .methods
                    .iter()
                    .any(|m| m.name == "<init>" && m.descriptor == "(LOwner;I)V")
            );
            assert_eq!(execute(&legacy, "Owner"), expected);
        }
    }
}

#[test]
fn ordinary_super_delegation_precedes_local_declarations() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let output = work.path().join("source");
    let recompiled = work.path().join("recompiled");
    compile(
        &[PathBuf::from(
            "tests/fixtures/java/ConstructorStraight.java",
        )],
        &original,
        None,
    );
    let expected = crate::common::execute(&original, "ConstructorStraight");
    crate::common::decompile(&original, &output);
    let source = fs::read_to_string(output.join("ConstructorStraight.java")).unwrap();
    assert!(
        !source.contains("$Preinit"),
        "ordinary constructors should remain ordinary:\n{source}"
    );
    compile(&files(&output, "java"), &recompiled, None);
    let actual = crate::common::execute(&recompiled, "ConstructorStraight");
    assert_eq!(actual, expected);
}

#[test]
fn post_delegation_catch_local_is_defined_by_its_handler() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let output = work.path().join("source");
    let recompiled = work.path().join("recompiled");
    compile(
        &[PathBuf::from("tests/fixtures/java/ConstructorCatch.java")],
        &original,
        None,
    );
    let path = original.join("ConstructorCatch.class");
    let mut bytes = fs::read(&path).unwrap();
    let class = sporeflower::classfile::ClassFile::parse(&bytes).unwrap();
    let method = class
        .methods
        .iter()
        .find(|method| method.name == "<init>")
        .unwrap();
    let code = &method.code.as_ref().unwrap().bytes;
    let instructions = sporeflower::bytecode::decode(code).unwrap();
    let delegation = instructions
        .iter()
        .position(|instruction| instruction.opcode == 0xb7)
        .unwrap();
    let before = instructions
        .iter()
        .position(|instruction| instruction.opcode == 0xb8)
        .unwrap();
    let start = instructions[delegation + 1].pc as usize;
    let end = instructions[before + 1].pc as usize;
    let reordered: Vec<_> = code[start..end]
        .iter()
        .chain(&code[..start])
        .copied()
        .collect();
    let offset = bytes
        .windows(code.len())
        .position(|window| window == code)
        .unwrap();
    bytes[offset..offset + end].copy_from_slice(&reordered);
    fs::write(path, bytes).unwrap();
    let expected = crate::common::execute(&original, "ConstructorCatch");
    crate::common::decompile(&original, &output);
    let source = fs::read_to_string(output.join("ConstructorCatch.java")).unwrap();
    assert!(
        source.contains("$Preinit"),
        "fixture must exercise constructor region extraction: {source}"
    );
    compile(&files(&output, "java"), &recompiled, None);
    let actual = crate::common::execute(&recompiled, "ConstructorCatch");
    assert_eq!(actual, expected);
}

#[test]
fn bounded_constructor_arguments_fall_back_to_a_carrier_with_valid_delegation() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from("tests/fixtures/java/ConstructorDepth.java")],
        &original,
        None,
    );
    crate::common::decompile(&original, &source);
    let emitted = fs::read_to_string(source.join("ConstructorDepth.java")).unwrap();
    assert!(emitted.contains("$preinit"), "{emitted}");
    assert!(
        emitted.contains("class $Preinit0_"),
        "referenced source type must not be shadowed: {emitted}"
    );
    compile(&files(&source, "java"), &rebuilt, None);
    assert_eq!(
        crate::common::execute(&original, "ConstructorDepth"),
        b"607\n29\n"
    );
    assert_eq!(
        crate::common::execute(&rebuilt, "ConstructorDepth"),
        b"607\n29\n"
    );
}

#[test]
fn unsupported_constructor_bodies_keep_accessible_delegation_without_new_checked_throws() {
    use sporeflower::{classfile::ClassFile, naming::NamePlan, program::Program};
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("FailureChild.java");
    fs::write(
        &source,
        r#"
class FailureBase {
    private FailureBase() {}
    FailureBase(String value) throws java.io.IOException {}
    FailureBase(int value) {}
    FailureBase(char value) {}
}
public class FailureChild extends FailureBase {
    public FailureChild() { super('x'); }
    public FailureChild(boolean ignored) { super(7); }
}
"#,
    )
    .unwrap();
    compile(std::slice::from_ref(&source), &original, None);
    let mut child =
        ClassFile::parse(&fs::read(original.join("FailureChild.class")).unwrap()).unwrap();
    let base = ClassFile::parse(&fs::read(original.join("FailureBase.class")).unwrap()).unwrap();
    for method in &mut child.methods {
        let code = method.code.as_mut().unwrap();
        if method.descriptor == "()V" {
            // Keep its exact char delegation recoverable, but leave an acquired
            // monitor at return so ordinary source reconstruction must fail.
            assert_eq!(code.bytes.pop(), Some(0xb1));
            code.bytes.extend([0x2a, 0xc2, 0xb1]);
        } else {
            // Lifting cannot recover this delegation. The fallback must skip
            // private super() and the String overload's undeclared IOException.
            code.bytes = vec![0xca];
        }
    }
    let classes = [child];
    let libraries = [base];
    let program = Program::new(&classes, &libraries).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let rendered =
        sporeflower::java::decompile_class(&program, &names, 0, &Default::default()).unwrap();
    assert_eq!(rendered.failed_methods, 2, "{:?}", rendered.diagnostics);
    assert!(
        rendered.source.contains("super((char) 0);"),
        "{}",
        rendered.source
    );
    assert!(rendered.source.contains("super(0);"), "{}", rendered.source);
    assert!(!rendered.source.contains("throws "), "{}", rendered.source);
    fs::write(&source, &rendered.source).unwrap();
    compile(
        std::slice::from_ref(&source),
        &work.path().join("rebuilt"),
        Some(&original),
    );

    // The lifted invocation still provides its exact descriptor when the
    // superclass itself was not supplied to the decompiler.
    let mut child = classes[0].clone();
    child.methods.retain(|method| method.descriptor == "()V");
    let classes = [child];
    let program = Program::new(&classes, &[]).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let rendered =
        sporeflower::java::decompile_class(&program, &names, 0, &Default::default()).unwrap();
    assert!(
        rendered.source.contains("super((char) 0);"),
        "{}",
        rendered.source
    );
    fs::write(&source, &rendered.source).unwrap();
    compile(
        &[source],
        &work.path().join("without-parent-metadata"),
        Some(&original),
    );
}

#[test]
fn inner_constructor_failure_uses_enclosing_static_support() {
    use sporeflower::{
        classfile::ClassFile, java::SourceUnits, naming::NamePlan, program::Program,
    };

    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("House.java");
    fs::write(
        &source,
        r#"
        class House {
            static class Base { Base() {} }
            class Room extends Base {
                Room(String name) {}
                House owner() { return House.this; }
            }
            Room open() { return new Room(null); }
        }
        class House$$sf$Support1$$ConstructorFailure0 {}
    "#,
    )
    .unwrap();
    compile(std::slice::from_ref(&source), &original, None);
    let read = |name: &str| {
        ClassFile::parse(&fs::read(original.join(format!("{name}.class"))).unwrap()).unwrap()
    };
    let mut room = read("House$Room");
    let ctor = room
        .methods
        .iter_mut()
        .find(|m| m.name == "<init>")
        .unwrap();
    let code = ctor.code.as_mut().unwrap();
    assert_eq!(code.bytes.pop(), Some(0xb1));
    code.bytes.extend([0x2a, 0xc2, 0xb1]); // unbalanced monitor after super()
    let classes = [
        read("House"),
        room,
        read("House$Base"),
        read("House$$sf$Support1$$ConstructorFailure0"),
    ];
    let program = Program::new(&classes, &[]).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let units = SourceUnits::new(&program, &names);
    assert_eq!(units.root(1), 0);
    let rendered = units
        .decompile(&program, &names, 0, &Default::default())
        .unwrap();
    assert_eq!(rendered.failed_classes, 0, "{:?}", rendered.diagnostics);
    assert_eq!(rendered.failed_methods, 1, "{:?}", rendered.diagnostics);
    // The support container must reserve its nested binary names. The hidden
    // House argument must not make the bridge collide with new Room(null).
    assert!(
        rendered
            .source
            .contains("new $sf$Support1_.$ConstructorFailure0(), false"),
        "{}",
        rendered.source
    );
    fs::write(&source, rendered.source).unwrap();
    let driver = work.path().join("Driver.java");
    fs::write(&driver, r#"class Driver {
        public static void main(String[] args) {
            try { new House().open(); }
            catch (Error e) { System.out.println(e.getMessage().indexOf("House$Room.<init>") >= 0); }
        }
    }"#).unwrap();
    let rebuilt = work.path().join("rebuilt");
    let files = [source, driver];
    compile(&files, &rebuilt, None);
    assert_eq!(crate::common::execute(&rebuilt, "Driver"), b"true\n");
    if crate::common::legacy_javac().is_some() {
        let api = sporeflower::api::ApiCatalog::bundled()
            .unwrap()
            .select(&classes, &Default::default())
            .unwrap();
        let jar = work.path().join("api.jar");
        api.write_jar(&jar).unwrap();
        let legacy = work.path().join("legacy");
        crate::common::legacy_compile(&files[..1], &legacy, &jar);
        fs::copy(rebuilt.join("Driver.class"), legacy.join("Driver.class")).unwrap();
        assert_eq!(crate::common::execute(&legacy, "Driver"), b"true\n");
    }
}

#[test]
fn inspection_retains_receiver_copy_used_after_delegation() {
    use crate::common::{execute, legacy_compile, sources};
    use sporeflower::classfile::ClassFile;

    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("ReceiverCopy.java");
    fs::write(
        &source,
        r#"
        class CopyBase {
            int base;
            CopyBase(int n) { base = n + 1; }
        }
        public class ReceiverCopy extends CopyBase {
            int field;
            ReceiverCopy(int n) {
                super(n);
                ReceiverCopy copy = this;
                copy.field = n;
            }
            public static void main(String[] args) {
                ReceiverCopy x = new ReceiverCopy(7);
                System.out.println(x.base + x.field);
            }
        }
        "#,
    )
    .unwrap();
    compile(&[source], &original, None);
    let class = original.join("ReceiverCopy.class");
    let mut bytes = fs::read(&class).unwrap();
    let parsed = ClassFile::parse(&bytes).unwrap();
    let code = &parsed
        .methods
        .iter()
        .find(|m| m.name == "<init>")
        .unwrap()
        .code
        .as_ref()
        .unwrap()
        .bytes;
    assert_eq!(&code[5..7], &[0x2a, 0x4d]); // aload_0, astore_2
    let at = bytes
        .windows(code.len())
        .position(|window| window == code)
        .unwrap();
    bytes[at..at + 7].rotate_right(2); // retain the alias across invokespecial
    fs::write(class, bytes).unwrap();
    let expected = execute(&original, "ReceiverCopy");
    assert_eq!(expected, b"15\n");
    for preserve in [false, true] {
        let output = work.path().join(format!("output-{preserve}"));
        let api = work.path().join("api.jar");
        let mut decompiler = Command::new(env!("CARGO_BIN_EXE_sporeflower"));
        decompiler
            .arg(&original)
            .arg(&output)
            .arg("--stubs")
            .arg(&api);
        if preserve {
            decompiler.arg("--preserve-bytecode");
        }
        checked(&mut decompiler);
        let emitted = fs::read_to_string(output.join("ReceiverCopy.java")).unwrap();
        assert!(
            !emitted.contains("Bytecode inspection used simplified Java"),
            "{emitted}"
        );
        if preserve {
            assert!(
                emitted.find("super(").unwrap() < emitted.find(" = this;").unwrap(),
                "{emitted}"
            );
        }
        let files = sources(&output);
        let rebuilt = output.join("classes");
        compile(&files, &rebuilt, None);
        assert_eq!(execute(&rebuilt, "ReceiverCopy"), expected);
        let legacy = output.join("legacy");
        if legacy_compile(&files, &legacy, &api) {
            assert_eq!(execute(&legacy, "ReceiverCopy"), expected);
        }
    }
}

#[test]
fn inspection_reports_when_distinct_super_calls_require_simplification() {
    use sporeflower::{
        classfile::ClassFile,
        java::{JavaOptions, decompile_class},
        naming::NamePlan,
        program::Program,
    };
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("InspectionChild.java");
    fs::write(
        &source,
        r#"
class InspectionBase {
    InspectionBase() {}
    InspectionBase(int ignored) {}
}
public class InspectionChild extends InspectionBase {
    public InspectionChild() { super(); }
    public InspectionChild(int ignored) { super(1); }
}
"#,
    )
    .unwrap();
    let original = work.path().join("original");
    compile(std::slice::from_ref(&source), &original, None);
    let parse = |file| ClassFile::parse(&fs::read(original.join(file)).unwrap()).unwrap();
    let mut child = parse("InspectionChild.class");
    let base = parse("InspectionBase.class");
    let body = |descriptor| {
        child
            .methods
            .iter()
            .find(|m| m.descriptor == descriptor)
            .unwrap()
            .code
            .as_ref()
            .unwrap()
            .bytes
            .clone()
    };
    let first = body("()V");
    let second = body("(I)V");
    let target = i16::try_from(3 + first.len()).unwrap().to_be_bytes();
    let code = child
        .methods
        .iter_mut()
        .find(|m| m.descriptor == "()V")
        .unwrap()
        .code
        .as_mut()
        .unwrap();
    // The JVM permits either delegation site. Java requires one first statement.
    code.bytes = [vec![0x04, 0x99, target[0], target[1]], first, second].concat();
    code.max_stack = 2;
    code.local_variables.clear();
    code.line_numbers.clear();
    let classes = [child];
    let libraries = [base];
    let program = Program::new(&classes, &libraries).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let options = JavaOptions {
        preserve_bytecode: true,
        ..Default::default()
    };
    let rendered = decompile_class(&program, &names, 0, &options).unwrap();
    assert_eq!(rendered.failed_methods, 0, "{:?}", rendered.diagnostics);
    assert_eq!(
        rendered.inspection_fallback_methods, 1,
        "{:?}",
        rendered.diagnostics
    );
    assert_eq!(rendered.simplifications.folded_decisions, 1);
    assert!(
        rendered
            .diagnostics
            .iter()
            .any(|message| message.contains("InspectionChild.<init>()V: used simplified Java"))
    );
    assert!(
        rendered
            .source
            .contains("// Bytecode inspection used simplified Java")
    );
    fs::write(&source, rendered.source).unwrap();
    compile(&[source], &work.path().join("rebuilt"), Some(&original));
}
