//! Authored end-to-end cases for mappings that carry meaning into source.
//! The input Java contains only literal values; generated names must preserve
//! both runtime behavior and the original identities in the name sidecar.
use crate::common::{checked, compile_command, execute, sources};
use sporeflower::{bytecode::Operand, classfile::ClassFile};
use std::{fs, io::Write, path::PathBuf, process::Command};

struct Case {
    _work: tempfile::TempDir,
    original: PathBuf,
    generated: PathBuf,
    rebuilt: PathBuf,
    mappings: PathBuf,
}

impl Case {
    fn new(java: &str) -> Self {
        let work = tempfile::tempdir().unwrap();
        let original = work.path().join("original");
        let generated = work.path().join("generated");
        let rebuilt = work.path().join("rebuilt");
        let mappings = work.path().join("mappings");
        fs::create_dir_all(&original).unwrap();
        fs::create_dir_all(&mappings).unwrap();
        let source = work.path().join("Probe.java");
        fs::write(&source, java).unwrap();
        checked(
            compile_command()
                .args(["-g:none", "-classpath"])
                .arg(&original)
                .arg("-d")
                .arg(&original)
                .arg(&source),
        );
        Self {
            _work: work,
            original,
            generated,
            rebuilt,
            mappings,
        }
    }

    fn render(&self, mapping: &str) {
        fs::write(self.mappings.join("Program.map"), mapping).unwrap();
        checked(
            Command::new(env!("CARGO_BIN_EXE_sporeflower"))
                .arg(&self.original)
                .arg("-o")
                .arg(&self.generated)
                .arg("--mappings")
                .arg(&self.mappings)
                .args(["--no-api", "--threads", "1"]),
        );
    }

    fn render_project_with_reports(&self, mapping: &str) -> PathBuf {
        let root = self._work.path();
        let archive = root.join("input.jar");
        let mut jar = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        for path in crate::common::files(&self.original, "class") {
            let name = path.strip_prefix(&self.original).unwrap().to_str().unwrap();
            jar.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            jar.write_all(&fs::read(path).unwrap()).unwrap();
        }
        jar.finish().unwrap();
        let config = "jar = \"input.jar\"\n";
        fs::write(root.join("j2me.toml"), config).unwrap();
        fs::write(self.mappings.join("Program.map"), mapping).unwrap();
        let original_archive = fs::read(&archive).unwrap();
        let reports = root.join("reports");
        checked(
            Command::new(env!("CARGO_BIN_EXE_sporeflower"))
                .arg("--project")
                .arg(root)
                .arg("-o")
                .arg(&self.generated)
                .arg("--mapping-report")
                .arg(&reports)
                .args(["--no-api", "--threads", "1"]),
        );
        assert_eq!(fs::read(&archive).unwrap(), original_archive);
        assert_eq!(fs::read_to_string(root.join("j2me.toml")).unwrap(), config);
        assert_eq!(
            fs::read_to_string(self.mappings.join("Program.map")).unwrap(),
            mapping
        );
        reports
    }

    fn source(&self, name: &str) -> String {
        fs::read_to_string(self.generated.join(format!("readable/{name}.java"))).unwrap()
    }

    fn roundtrip(&self) {
        crate::common::compile(&sources(&self.generated), &self.rebuilt, None);
        assert_eq!(
            execute(&self.original, "raw.Probe"),
            execute(&self.rebuilt, "readable.Probe"),
            "mappings changed the authored program's runtime trace",
        );
    }

    fn invoke_offset(&self, method_name: &str, descriptor: &str, called: &str, nth: usize) -> u32 {
        self.invoke_offset_in("raw/a", method_name, descriptor, "raw/a", called, nth)
    }

    fn invoke_offset_in(
        &self,
        owner: &str,
        method_name: &str,
        descriptor: &str,
        called_owner: &str,
        called: &str,
        nth: usize,
    ) -> u32 {
        let class =
            ClassFile::parse(&fs::read(self.original.join(format!("{owner}.class"))).unwrap())
                .unwrap();
        let method = class
            .methods
            .iter()
            .find(|m| m.name == method_name && m.descriptor == descriptor)
            .unwrap();
        sporeflower::bytecode::decode(&method.code.as_ref().unwrap().bytes)
            .unwrap()
            .iter()
            .filter(|instruction| matches!(instruction.opcode, 0xb6..=0xb9))
            .filter(|instruction| {
                let index = match instruction.operand {
                    Operand::Constant(index) | Operand::InterfaceCall { index, .. } => index,
                    _ => return false,
                };
                let member = class.constant_pool.member_ref(index).unwrap();
                member.owner == called_owner && member.name == called
            })
            .nth(nth)
            .unwrap()
            .pc
    }
}

fn contains(source: &str, expected: &str) {
    assert!(
        source.contains(expected),
        "missing {expected:?} in:\n{source}"
    );
}

#[test]
fn domain_constants_take_precedence_over_standard_integer_limits() {
    let case = Case::new(
        r#"
package raw;
class a {
    static final int bound = 2147483647;
    static int value(boolean high) { return high ? 2147483647 : -2147483648; }
    static int selected(int value) {
        switch (value) {
            case 2147483647: return 1;
            case -2147483648: return 2;
            default: return 3;
        }
    }
}
public class Probe {
    public static void main(String[] args) {
        System.out.println(a.value(true) + ":" + a.value(false) + ":" + a.selected(a.value(true)));
    }
}
"#,
    );
    case.render(
        r#"
package readable;
@ValueDomain interface Bound { int UNBOUNDED = 2147483647; int UNSET = -2147483648; }
class Limits /* was raw/a */ {
    @Domain(Bound.class) static final int bound /* was bound */;
    @Domain(Bound.class) int value(boolean high) /* was value */;
    int selected(@Domain(Bound.class) int value) /* was selected */;
}
class Probe /* was raw/Probe */ {}
"#,
    );
    let source = case.source("Limits");
    contains(&source, "bound = Bound.UNBOUNDED");
    contains(&source, "Bound.UNSET");
    contains(&source, "case Bound.UNBOUNDED:");
    contains(&source, "case Bound.UNSET:");
    assert!(
        !source.contains("Integer.MAX_VALUE") && !source.contains("Integer.MIN_VALUE"),
        "{source}"
    );
    case.roundtrip();
}

#[test]
fn merged_packages_preserve_each_runtime_class_resource_directory() {
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("input");
    let original = work.path().join("original");
    let generated = work.path().join("generated");
    let rebuilt = work.path().join("rebuilt");
    let maps = work.path().join("maps");
    fs::create_dir_all(&maps).unwrap();
    for package in ["one", "two"] {
        let directory = source.join(package);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("Reader.java"), format!(
            "package {package}; public class Reader {{ public int read() throws java.io.IOException {{ return getClass().getResourceAsStream(\"data.txt\").read(); }} }}"
        )).unwrap();
    }
    fs::create_dir_all(source.join("entry")).unwrap();
    fs::write(
        source.join("entry/Main.java"),
        r#"
package entry;
public class Main {
    public static void main(String[] args) throws Exception {
        System.out.println(new one.Reader().read());
        System.out.println(new two.Reader().read());
    }
}
"#,
    )
    .unwrap();
    crate::common::compile(&sources(&source), &original, None);
    for (package, content) in [("one", "A"), ("two", "B")] {
        fs::write(original.join(package).join("data.txt"), content).unwrap();
    }
    fs::write(
        maps.join("Readers.map"),
        r#"
package merged;
class FirstReader /* was one/Reader */ {}
class SecondReader /* was two/Reader */ {}
"#,
    )
    .unwrap();
    fs::write(
        maps.join("Main.map"),
        "package app; class Entry /* was entry/Main */ {}",
    )
    .unwrap();
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg("-o")
            .arg(&generated)
            .arg("--mappings")
            .arg(&maps)
            .args(["--no-api", "--threads", "1"]),
    );
    crate::common::compile(&sources(&generated), &rebuilt, None);
    for (package, content) in [("one", "A"), ("two", "B")] {
        fs::create_dir_all(rebuilt.join(package)).unwrap();
        fs::write(rebuilt.join(package).join("data.txt"), content).unwrap();
    }
    assert_eq!(
        execute(&original, "entry.Main"),
        execute(&rebuilt, "app.Entry")
    );
}

#[test]
fn mapped_override_families_parameters_and_original_descriptors_roundtrip() {
    let case = Case::new(
        r#"
package raw;
interface a { int a(int x, long y, int z); }
class b implements a {
    int a;
    b(int x) { a = x; }
    public int a(int x, long y, int z) { return a + x + (int)y + z; }
    int a(String x) { return x.length(); }
}
class c extends b {
    c(int x) { super(x); }
    public int a(int x, long y, int z) { return super.a(x, y, z) + 1; }
}
public class Probe {
    public static void main(String[] args) {
        a operation = new c(7);
        System.out.println(operation.a(3, 5L, 2));
        System.out.println(new b(4).a("abc"));
    }
}
"#,
    );
    case.render(
        r#"
package readable;
interface Operation /* was raw/a */ {
    int apply(int delta, long elapsed, int limit) /* was a */;
}
class Base /* was raw/b */ {
    int seed /* was a */;
    Base(int seed) {}
    int length(String text) /* was a */;
}
class Child extends Base /* was raw/c */ { Child(int seed) {} }
class Probe /* was raw/Probe */ {}
"#,
    );
    contains(
        &case.source("Operation"),
        "apply(int delta, long elapsed, int limit)",
    );
    contains(&case.source("Base"), "length(String text)");
    contains(&case.source("Base"), "Base(int seed)");
    contains(&case.source("Child"), "apply(");
    contains(&case.source("Child"), "super.apply(");
    let tiny = fs::read_to_string(case.generated.join("mapping.tiny")).unwrap();
    contains(&tiny, "c\traw/b\treadable/Base");
    contains(&tiny, "m\t(IJI)I\ta\tapply");
    contains(&tiny, "m\t(Ljava/lang/String;)I\ta\tlength");
    contains(&tiny, "p\t1\t\tdelta");
    contains(&tiny, "p\t2\t\telapsed");
    contains(&tiny, "p\t4\t\tlimit");
    case.roundtrip();
}

#[test]
fn interface_fields_share_analysis_without_sharing_helper_requirements_or_element_domains() {
    let case = Case::new(
        r#"
package raw;
interface Data {
    int[] states = new int[]{1, 2};
    java.io.InputStream resource = Probe.class.getResourceAsStream("missing.bin");
    byte[] bytes = new byte[]{1, 2};
    short[] shorts = new short[]{1, 2};
    int[] levels = new int[]{1, 2};
    long[] longs = new long[]{1L, 2L};
    boolean[] flags = new boolean[]{false, true};
}
public class Probe {
    public static void main(String[] args) {
        System.out.println(Data.states[0] + Data.states[1]);
        System.out.println(Data.resource == null);
        System.out.println(Data.bytes[0] + Data.shorts[1] + Data.levels[0] + Data.longs[1]);
        System.out.println(Data.flags[0]);
        System.out.println(Data.flags[1]);
    }
}
"#,
    );
    case.render(
        r#"
package readable;
@ValueDomain interface State { int OFF = 1; int ON = 2; }
@ValueDomain interface Level { int LOW = 1; int HIGH = 2; }
interface Data /* was raw/Data */ {
    @Domain(State.class) int[] states /* was states */;
    @Domain(State.class) byte[] bytes /* was bytes */;
    @Domain(Level.class) short[] shorts /* was shorts */;
    @Domain(Level.class) int[] levels /* was levels */;
    @Domain(State.class) long[] longs /* was longs */;
}
class Probe /* was raw/Probe */ {}
"#,
    );
    let source = case.source("Data");
    contains(&source, "states = new int[] { State.OFF, State.ON }");
    contains(&source, "resource = $Init.value1()");
    contains(&source, "$sf$resource(");
    contains(&source, "bytes = new byte[] { State.OFF, State.ON }");
    contains(&source, "shorts = new short[] { Level.LOW, Level.HIGH }");
    contains(&source, "levels = new int[] { Level.LOW, Level.HIGH }");
    contains(
        &source,
        "longs = new long[] { (long) State.OFF, (long) State.ON }",
    );
    contains(&source, "flags = new boolean[] { false, true }");
    case.roundtrip();
}

#[test]
fn real_constants_branch_values_and_exclusive_flags_preserve_arithmetic() {
    let case = Case::new(
        r#"
package raw;
class a {
    public static final int a = 5;
    public static final int b = 8;
    int c;
    a(int value) { c = value; }
    int a(int value) { c = value; return c; }
    int b(int value) { return value; }
    int c(int value) { return value - 5; }
}
public class Probe {
    public static void main(String[] args) {
        a device = new a(5);
        int state;
        if (args.length == 0) state = 5; else state = 8;
        System.out.println(device.a(state));
        System.out.println(device.b(37));
        System.out.println(device.b(39));
        System.out.println(device.c(8));
    }
}
"#,
    );
    let reports = case.render_project_with_reports(
        r#"
package readable;
@ValueDomain interface State {}
@FlagDomain(exclusiveMasks = {3}) interface Options {
    int SINGLE = 1;
    int DOUBLE = 2;
    int ENABLED = 4;
}
class Device /* was raw/a */ {
    @DomainValue(State.class) static final int READY /* was a */;
    @DomainValue(State.class) static final int BUSY /* was b */;
    @Domain(State.class) int state /* was c */;
    Device(@Domain(State.class) int state) {}
    @Domain(State.class) int setState(@Domain(State.class) int state) /* was a */;
    @Flags(Options.class) int options(@Flags(Options.class) int options) /* was b */;
    int distance(@Domain(State.class) int state) /* was c */;
}
class Probe /* was raw/Probe */ {}
"#,
    );
    let coverage = fs::read_to_string(reports.join("coverage.md")).unwrap();
    contains(&coverage, "raw/a");
    contains(&coverage, "readable/Device");
    let usage = fs::read_to_string(reports.join("usage-priority.tsv")).unwrap();
    contains(&usage, "raw/a\ta\t(I)I\treadable/Device\tsetState");
    let summary = fs::read_to_string(reports.join("semantic-summary.md")).unwrap();
    contains(&summary, "readable.State");
    contains(&summary, "raw/a.a(I)I");
    let source = case.source("Probe");
    contains(&source, "Device.READY");
    contains(&source, "Device.BUSY");
    contains(&source, "Options.SINGLE");
    contains(&source, "Options.ENABLED");
    assert!(
        !source.contains("Options.DOUBLE"),
        "exclusive choice 3 must remain numeric:\n{source}"
    );
    contains(&case.source("Device"), "- 5");
    assert!(!case.source("Device").contains("- READY"));
    assert!(
        !case.generated.join("readable/State.java").exists(),
        "real constants do not need a generated empty interface"
    );
    case.roundtrip();
}

#[test]
fn record_aliases_and_consumer_typed_local_rows_keep_separate_column_meanings() {
    let case = Case::new(
        r#"
package raw;
class a {
    static int[] a = new int[]{1, 2, 2, 1};
    static int a(int[] row) { return row[0] + row[1]; }
    static int a(int index) {
        int[] row = new int[2];
        row[0] = 2;
        row[1] = 1;
        int[] alias = a;
        int state = alias[(index & 255) * 2 + 1];
        return a(row) + (state == 2 ? 10 : 20);
    }
}
public class Probe {
    public static void main(String[] args) {
        System.out.println(a.a(0));
        System.out.println(a.a(1));
    }
}
"#,
    );
    case.render(
        r#"
package readable;
@ValueDomain interface Kind { int FIRST = 1; int SECOND = 2; }
@ValueDomain interface State { int WAITING = 1; int READY = 2; }
@SlotDomain interface Pair {
    @SlotValue(Kind.class) int KEY = 0;
    @SlotValue(State.class) int VALUE = 1;
}
class Tables /* was raw/a */ {
    @Records(value = Pair.class, stride = 2) int[] pairs /* was a */;
    int consume(@Slots(Pair.class) int[] row) /* was a */;
    int lookup(int index) /* was a */;
}
class Probe /* was raw/Probe */ {}
"#,
    );
    let source = case.source("Tables");
    contains(&source, "Pair.KEY");
    contains(&source, "Pair.VALUE");
    contains(&source, "Kind.SECOND");
    contains(&source, "State.WAITING");
    contains(&source, "State.READY");
    let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        compact.contains("newint[]{Kind.SECOND,State.WAITING}")
            || (compact.contains("[Pair.KEY]=Kind.SECOND")
                && compact.contains("[Pair.VALUE]=State.WAITING")),
        "the local row must retain column meanings, including after initializer folding:\n{source}"
    );
    case.roundtrip();
}

#[test]
fn conditional_arguments_and_bytecode_scoped_calls_do_not_type_generic_calls() {
    let case = Case::new(
        r#"
package raw;
class a {
    static int d = 7;
    static int a(int value) { return value; }
    static int b(int selector, int payload) { return selector + payload; }
    static int c() { return d; }
    static int a() {
        int scoped = a(7);
        int ordinary = a(7);
        return scoped + ordinary + b(1, 2) + b(2, 2) + (c() == 7 ? 100 : 200);
    }
}
public class Probe {
    public static void main(String[] args) {
        System.out.println(a.a());
        a.d = 8;
        System.out.println(a.a());
    }
}
"#,
    );
    let argument = case.invoke_offset("a", "()I", "a", 0);
    let result = case.invoke_offset("a", "()I", "c", 0);
    case.render(&format!(r#"
package readable;
@ValueDomain interface Opcode {{ int EVENT = 7; }}
@ValueDomain interface State {{ int READY = 2; }}
class Decoder /* was raw/a */ {{
    int generic(int value) /* was a */;
    int selected(int selector, @DomainWhen(value = State.class, parameter = 0, equals = 1) int payload) /* was b */;
    int read() /* was c */;
    @CallDomain(value = Opcode.class, offset = {argument}, parameter = 0)
    @CallDomain(value = Opcode.class, offset = {result})
    int run() /* was a */;
}}
class Probe /* was raw/Probe */ {{}}
"#));
    let source = case.source("Decoder");
    contains(&source, "generic(Opcode.EVENT)");
    contains(&source, "generic(7)");
    contains(&source, "selected(1, State.READY)");
    contains(&source, "selected(2, 2)");
    assert!(
        source.contains("read() == Opcode.EVENT") || source.contains("read() != Opcode.EVENT"),
        "the scoped call result must name its comparison:\n{source}"
    );
    assert!(
        !source.contains("return Opcode.EVENT;"),
        "a scoped result must not bind the generic helper:\n{source}"
    );
    case.roundtrip();
}

#[test]
fn scoped_constructor_arguments_survive_this_and_super_delegation() {
    let case = Case::new(
        r#"
package raw;
class a {
    int v;
    a(int value) { v = value; }
    a() { this(7); }
}
class b extends a { b() { super(7); } }
public class Probe {
    public static void main(String[] args) {
        System.out.println(new a().v);
        System.out.println(new b().v);
        System.out.println(new a(7).v);
    }
}
"#,
    );
    let this_call = case.invoke_offset_in("raw/a", "<init>", "()V", "raw/a", "<init>", 0);
    let super_call = case.invoke_offset_in("raw/b", "<init>", "()V", "raw/a", "<init>", 0);
    case.render(&format!(
        r#"
package readable;
@ValueDomain interface Mode {{ int ACTIVE = 7; }}
class Device /* was raw/a */ {{
    int value /* was v */;
    Device(int value) {{}}
    @CallDomain(value = Mode.class, offset = {this_call}, parameter = 0)
    Device() {{}}
}}
class Child /* was raw/b */ extends Device {{
    @CallDomain(value = Mode.class, offset = {super_call}, parameter = 0)
    Child() {{}}
}}
class Probe /* was raw/Probe */ {{}}
"#
    ));
    contains(&case.source("Device"), "this(Mode.ACTIVE)");
    contains(&case.source("Child"), "super(Mode.ACTIVE)");
    contains(&case.source("Probe"), "new Device(7)");
    case.roundtrip();
}

#[test]
fn packed_long_fields_and_signed_coordinate_slices_keep_widths_and_local_names() {
    let case = Case::new(
        r#"
package raw;
class a {
    static long a(int cost, int parent) { return (long)cost << 16 | (long)parent << 32; }
    static int a(long value) {
        int cost = (int)(value >>> 16 & 65535L);
        int parent = (int)(value >>> 32 & 255L);
        System.out.println(cost);
        System.out.println(parent);
        return cost + parent;
    }
    static int b(int point) {
        if (point == -1) return 0;
        int x = point >> 16;
        int y = (short)point;
        System.out.println(x);
        System.out.println(y);
        return x + y;
    }
}
public class Probe {
    public static void main(String[] args) {
        System.out.println(a.a(a.a(60000, 200)));
        System.out.println(a.b((-2 << 16) | 65533));
        System.out.println(a.b(-1));
    }
}
"#,
    );
    case.render(
        r#"
package readable;
@PackedDomain
@BitField(name = "cost", shift = 16, bits = 16)
@BitField(name = "parent", shift = 32, bits = 8)
interface NodeWord {}
@ValueDomain
@BitField(name = "x", shift = 16, bits = 16, signed = true)
@BitField(name = "y", bits = 16, signed = true)
interface Point { int NONE = -1; }
class Packing /* was raw/a */ {
    @Domain(NodeWord.class) long pack(int cost, int parent) /* was a */;
    int unpack(@Domain(NodeWord.class) long word) /* was a */;
    int coordinateSum(@Domain(Point.class) int point) /* was b */;
}
class Probe /* was raw/Probe */ {}
"#,
    );
    let source = case.source("Packing");
    contains(&source, "NodeWord.COST_SHIFT");
    contains(&source, "NodeWord.PARENT_SHIFT");
    contains(&source, "NodeWord.COST_VALUE_MASK");
    contains(&source, "NodeWord.PARENT_VALUE_MASK");
    contains(&source, "Point.NONE");
    contains(&source, "Point.X_SHIFT");
    contains(&source, "int cost =");
    contains(&source, "int parent =");
    contains(&source, "int x =");
    contains(&source, "short y =");
    case.roundtrip();
}

#[test]
fn exact_string_tokens_and_numeric_formats_preserve_stored_values() {
    let case = Case::new(
        r#"
package raw;
class a {
    int a = 0x123456;
    int b = 384;
    int c = 1500;
    String d = "GET";
    boolean a() { return d.equals("GET"); }
    String b() { return "GET"; }
}
public class Probe {
    public static void main(String[] args) {
        a values = new a();
        System.out.println(values.a);
        System.out.println(values.b);
        System.out.println(values.c);
        System.out.println(values.a());
        System.out.println(values.b());
    }
}
"#,
    );
    case.render(
        r#"
package readable;
@NumericDomain(format = "rgb") interface Color {}
@NumericDomain(format = "fixed", fractionBits = 8) interface Fixed8 {}
@NumericDomain(format = "scaled", divisor = 1000, unit = "px") interface Millipixels {}
@StringDomain interface Token { String READ = "GET"; }
class Values /* was raw/a */ {
    @Domain(Color.class) int color /* was a */;
    @Domain(Fixed8.class) int position /* was b */;
    @Domain(Millipixels.class) int distance /* was c */;
    @Domain(Token.class) String token /* was d */;
    boolean isRead() /* was a */;
    String ordinaryText() /* was b */;
}
class Probe /* was raw/Probe */ {}
"#,
    );
    let source = case.source("Values");
    contains(&source, "0x123456");
    contains(&source, "Q8: 1.5");
    contains(&source, "/1000: 1.5 px");
    contains(&source, "Token.READ");
    contains(&source, "return \"GET\";");
    case.roundtrip();
}

#[test]
fn class_name_tables_helpers_and_direct_lookups_follow_package_moves() {
    let case = Case::new(
        r#"
package raw;
class a {}
class b {
    static String[] a = new String[]{"raw.a", "raw.b"};
    static Class a(String name) throws ClassNotFoundException { return Class.forName(name); }
}
public class Probe {
    public static void main(String[] args) throws Exception {
        System.out.println(b.a(b.a[0]) == a.class);
        System.out.println(b.a("raw.b") == b.class);
        System.out.println(Class.forName("raw.a") == a.class);
        System.out.println("raw.a");
    }
}
"#,
    );
    case.render(
        r#"
package readable;
class Payload /* was raw/a */ {}
class Registry /* was raw/b */ {
    @ClassName String[] typeNames /* was a */;
    Class lookup(@ClassName String name) /* was a */;
}
class Probe /* was raw/Probe */ {}
"#,
    );
    contains(&case.source("Registry"), "\"readable.Payload\"");
    contains(&case.source("Registry"), "\"readable.Registry\"");
    let source = case.source("Probe");
    contains(&source, "lookup(\"readable.Registry\")");
    contains(&source, "Class.forName(\"readable.Payload\")");
    contains(&source, "println(\"raw.a\")");
    case.roundtrip();
}

#[test]
fn inferred_lookup_wrappers_preserve_shared_text_in_source_and_native_remapping() {
    let case = Case::new(
        r#"
package raw;
class a {}
class b {
    static Class legacy(String name) {
        try { return Class.forName(name); }
        catch (ClassNotFoundException failure) {
            throw new NoClassDefFoundError(failure.getMessage());
        }
    }
    static Class forward(long before, String name, int after) {
        String alias = name;
        if (alias == null) return null;
        return legacy(alias);
    }
    final Class instance(String name) { return forward(0L, name, 0); }
    static Class initialized(boolean init, String name, ClassLoader loader) throws Exception {
        return Class.forName(name, init, loader);
    }
}
public class Probe {
    public static void main(String[] args) throws Exception {
        String shared = "raw.a";
        System.out.println(shared);
        System.out.println(b.forward(1L, shared, 2) == a.class);
        System.out.println(b.legacy("raw.b") == b.class);
        System.out.println(new b().instance("raw.a") == a.class);
        System.out.println(b.initialized(false, "raw.a", Probe.class.getClassLoader()) == a.class);
        System.out.println(b.forward(0L, null, 0) == null);
        String[] names = new String[] {
            "raw.a", "[Lraw.a;", "[[Lraw.a;", "[I", "raw/a", "[Lraw/a;", "raw.missing"
        };
        for (int i = 0; i < names.length; i++) {
            System.out.println(names[i]);
            try {
                Class type = b.forward(0L, names[i], 0);
                System.out.println(type.isArray() || type == a.class);
            } catch (NoClassDefFoundError expected) { System.out.println("missing"); }
        }
    }
}
"#,
    );
    let mapping = r#"
package readable;
class Payload /* was raw/a */ {}
class Registry /* was raw/b */ {}
class Probe /* was raw/Probe */ {}
"#;
    case.render(mapping);
    let source = case.source("Probe");
    contains(&source, "legacy(\"readable.Registry\")");
    contains(&source, "instance(\"readable.Payload\")");
    contains(&source, "$sf$className");
    case.roundtrip();

    let input = sporeflower::input::Input::read_for_remap(&case.original).unwrap();
    let program = sporeflower::program::Program::from_input(&input, &[]).unwrap();
    let mappings = sporeflower::mappings::parse_sources(
        &program,
        &[sporeflower::mappings::MappingSource::new(
            "lookup.map",
            mapping,
        )],
    )
    .unwrap();
    let names = sporeflower::naming::NamePlan::build_with_overrides(
        &program,
        &sporeflower::naming::RenameOptions::default(),
        &mappings.names,
    )
    .unwrap();
    let mapped = case._work.path().join("mapped.jar");
    sporeflower::remap::remap_jar_with_mappings(&mapped, &program, &names, &mappings).unwrap();
    assert_eq!(
        execute(&case.original, "raw.Probe"),
        execute(&mapped, "readable.Probe")
    );
}

#[test]
fn lookup_contract_inference_rejects_observed_parameters_and_virtual_dispatch() {
    let case = Case::new(
        r#"
package raw;
public class Probe {
    static String saved;
    static Class logged(String name) throws Exception {
        System.out.println(name); return Class.forName(name);
    }
    static Class locked(String name) throws Exception {
        synchronized (name) { return Class.forName(name); }
    }
    static Class published(String name) throws Exception {
        saved = name; return Class.forName(name);
    }
    static Class compared(String name) throws Exception {
        if (name == "raw.Probe") return Class.forName(name);
        return null;
    }
    static Class mixed(String first, String second) throws Exception {
        System.out.println(first); return Class.forName(second);
    }
    Class virtual(String name) throws Exception { return Class.forName(name); }
    static Class forwarded(Probe receiver, String name) throws Exception {
        return receiver.virtual(name);
    }
    public static void main(String[] args) {}
}
"#,
    );
    let input = sporeflower::input::Input::read_for_remap(&case.original).unwrap();
    let program = sporeflower::program::Program::from_input(&input, &[]).unwrap();
    let mappings = sporeflower::mappings::MappingPlan::default();
    let accepts = |name: &str, descriptor: &str, index| {
        sporeflower::semantics::class_names::is_argument(
            &program,
            &mappings,
            &sporeflower::ir::Member {
                owner: "raw/Probe".into(),
                name: name.into(),
                descriptor: descriptor.into(),
            },
            index,
        )
    };
    for name in ["logged", "locked", "published", "compared", "virtual"] {
        assert!(
            !accepts(name, "(Ljava/lang/String;)Ljava/lang/Class;", 0),
            "{name}"
        );
    }
    assert!(!accepts(
        "forwarded",
        "(Lraw/Probe;Ljava/lang/String;)Ljava/lang/Class;",
        1
    ));
    assert!(!accepts(
        "mixed",
        "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/Class;",
        0
    ));
    assert!(accepts(
        "mixed",
        "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/Class;",
        1
    ));
}

#[test]
fn header_and_parallel_planes_supply_per_call_helper_meanings() {
    let case = Case::new(
        r#"
package raw;
class a {
    static int[] a = new int[]{2, 11, 12, 0, 0, 1, 2, 0, 0};
    static int a(int[] table, int key) {
        for (int i = 0; i < 2; i++) if (table[1 + i] == key) return table[5 + i];
        return -1;
    }
    static int b(int index) { return a[1 + (index & 3) + 4]; }
    static int c() { return a[0]; }
    static boolean d() { return a(a, 11) == 2; }
}
public class Probe {
    public static void main(String[] args) {
        System.out.println(a.b(1));
        System.out.println(a.c());
        System.out.println(a.d());
    }
}
"#,
    );
    case.render(r#"
package readable;
@ValueDomain interface Key { int FIRST = 11; int SECOND = 12; }
@ValueDomain interface State { int MISSING = -1; int WAITING = 1; int READY = 2; }
@SlotDomain interface Header { int COUNT = 0; }
@SlotDomain interface Plane {
    @SlotValue(Key.class) int KEY = 0;
    @SlotValue(State.class) int VALUE = 1;
}
class Tables /* was raw/a */ {
    @Slots(Header.class) @Planes(value = Plane.class, stride = 4, offset = 1) int[] data /* was a */;
    @DomainFromSlot(parameter = 0, slot = 1)
    int lookup(int[] table, @DomainFromSlot(parameter = 0, slot = 0) int key) /* was a */;
    int valueAt(int index) /* was b */;
    int size() /* was c */;
    boolean ready() /* was d */;
}
class Probe /* was raw/Probe */ {}
"#);
    let source = case.source("Tables");
    contains(&source, "Header.COUNT");
    contains(&source, "Plane.VALUE");
    contains(&source, "Key.FIRST");
    contains(&source, "State.READY");
    // The helper has no fixed layout; its constants describe a generic algorithm.
    contains(&source, "table[5 +");
    case.roundtrip();
}

#[test]
fn boxed_container_contents_and_identity_helpers_leave_sizes_numeric() {
    let case = Case::new(
        r#"
package raw;
import java.util.Vector;
import java.util.Hashtable;
class a {
    static Vector a = new Vector();
    static Hashtable b = new Hashtable();
    static int b(int value) { return value; }
    static boolean a() {
        a.addElement(new Integer(2));
        b.put("GET", new Integer(1));
        return a.contains(new Integer(2))
            && ((Integer)b.get("GET")).intValue() == 1
            && b(((Integer)a.elementAt(0)).intValue()) == 2
            && a.size() == 1;
    }
}
public class Probe { public static void main(String[] args) { System.out.println(a.a()); } }
"#,
    );
    case.render(
        r#"
package readable;
@ValueDomain interface Kind { int FIRST = 1; int SECOND = 2; }
@StringDomain interface Token { String READ = "GET"; }
class Contents /* was raw/a */ {
    @Elements(Kind.class) java.util.Vector kinds /* was a */;
    @Keys(Token.class) @Values(Kind.class) java.util.Hashtable byToken /* was b */;
    @DomainFromParameter(0) int identity(int value) /* was b */;
    boolean check() /* was a */;
}
class Probe /* was raw/Probe */ {}
"#,
    );
    let source = case.source("Contents");
    contains(&source, "new Integer(Kind.SECOND)");
    contains(&source, "Token.READ");
    contains(&source, "Kind.FIRST");
    contains(&source, "elementAt(0)");
    contains(&source, "size() == 1");
    case.roundtrip();
}

#[test]
fn builtin_api_constants_use_the_available_api_and_can_be_disabled_independently() {
    let work = tempfile::tempdir().unwrap();
    let library_source = work.path().join("Graphics.java");
    let library = work.path().join("library");
    fs::write(
        &library_source,
        r#"
package javax.microedition.lcdui;
public class Graphics {
    public static final int HCENTER = 1, VCENTER = 2, LEFT = 4, RIGHT = 8,
        TOP = 16, BOTTOM = 32, BASELINE = 64;
    public int anchor, color;
    public void drawString(String text, int x, int y, int flags) { anchor = flags; }
    public void setColor(int rgb) { color = rgb; }
}

// The graphics contract pack also includes signatures that use Image.
class Image {}
"#,
    )
    .unwrap();
    crate::common::compile(&[library_source], &library, None);
    let input = work.path().join("Probe.java");
    fs::write(
        &input,
        r#"
package raw;
import javax.microedition.lcdui.Graphics;
public class Probe {
    static int a = 3;
    public static void main(String[] args) {
        Graphics graphics = new Graphics();
        graphics.drawString("x", 1, 2, 20);
        graphics.setColor(0x123456);
        System.out.println(graphics.anchor + graphics.color + a);
    }
}
"#,
    )
    .unwrap();
    let original = work.path().join("original");
    crate::common::compile(&[input], &original, Some(&library));
    let mapping = work.path().join("Program.map");
    fs::write(
        &mapping,
        r#"
package readable;
@ValueDomain interface Mode { int ACTIVE = 3; }
class Probe /* was raw/Probe */ { @Domain(Mode.class) int mode /* was a */; }
"#,
    )
    .unwrap();
    let run = |classes: &std::path::Path, name: &str| {
        let classpath = std::env::join_paths([classes, library.as_path()]).unwrap();
        checked(
            Command::new(crate::common::java())
                .args(["-Xverify:all", "-classpath"])
                .arg(classpath)
                .arg(name),
        )
        .stdout
    };
    let expected = run(&original, "raw.Probe");
    for (label, disabled) in [("builtin", false), ("project-only", true)] {
        let generated = work.path().join(label);
        let mut command = Command::new(env!("CARGO_BIN_EXE_sporeflower"));
        command
            .arg(&original)
            .arg("-o")
            .arg(&generated)
            .arg("--classpath")
            .arg(&library)
            .arg("--mappings")
            .arg(&mapping)
            .args(["--no-api", "--threads", "1"]);
        if disabled {
            command.arg("--no-semantic-mappings");
        }
        checked(&mut command);
        let source = fs::read_to_string(generated.join("readable/Probe.java")).unwrap();
        contains(&source, "Mode.ACTIVE");
        if disabled {
            assert!(!source.contains("Graphics.TOP"), "{source}");
            contains(&source, ", 20)");
        } else {
            contains(&source, "Graphics.TOP");
            contains(&source, "Graphics.LEFT");
            contains(&source, "0x123456");
        }
        let rebuilt = work.path().join(format!("{label}-classes"));
        crate::common::compile(&sources(&generated), &rebuilt, Some(&library));
        assert_eq!(run(&rebuilt, "readable.Probe"), expected);
    }
}

#[test]
fn field_and_array_updates_keep_target_meanings_and_index_names() {
    let case = Case::new(
        r#"
package raw;
class a {
    static int a = 1;
    static int[] b = new int[]{1, 1};
    static boolean a() { return a++ == 1; }
    static boolean b() { return ++b[1] == 2; }
    static int c(int value) { return value; }
    static int d() { return c(b[0]++); }
}
public class Probe {
    public static void main(String[] args) {
        System.out.println(a.a());
        System.out.println(a.b());
        System.out.println(a.d());
        System.out.println(a.a);
        System.out.println(a.b[0]);
        System.out.println(a.b[1]);
    }
}
"#,
    );
    case.render(
        r#"
package readable;
@ValueDomain interface Count { int ONE = 1; int TWO = 2; }
@SlotDomain interface CounterSlot {
    @SlotValue(Count.class) int LEFT = 0;
    @SlotValue(Count.class) int RIGHT = 1;
}
class Counters /* was raw/a */ {
    @Domain(Count.class) int value /* was a */;
    @Slots(CounterSlot.class) int[] values /* was b */;
    boolean oldValue() /* was a */;
    boolean newValue() /* was b */;
    int consume(@Domain(Count.class) int value) /* was c */;
    int argument() /* was d */;
}
class Probe /* was raw/Probe */ {}
"#,
    );
    let source = case.source("Counters");
    contains(&source, "value++");
    contains(&source, "[CounterSlot.RIGHT]");
    contains(&source, "[CounterSlot.LEFT]++");
    contains(&source, "Count.ONE");
    contains(&source, "Count.TWO");
    case.roundtrip();
}

#[test]
fn branch_class_names_used_as_string_receivers_or_reference_comparisons_stay_text() {
    let case = Case::new(
        r#"
package raw;
class a {}
class b {}
public class Probe {
    static int length(boolean first, boolean reflect) throws ClassNotFoundException {
        String name;
        if (first) name = "raw.a"; else name = "raw.b";
        if (reflect) Class.forName(name);
        return name.length();
    }
    static boolean identity(boolean first, boolean reflect) throws ClassNotFoundException {
        String name;
        if (first) name = "raw.a"; else name = "raw.b";
        if (reflect) Class.forName(name);
        return name == "raw.a";
    }
    public static void main(String[] args) throws Exception {
        System.out.println(length(true, false));
        System.out.println(length(false, false));
        System.out.println(identity(true, false));
        System.out.println(identity(false, false));
        System.out.println(length(true, true));
        System.out.println(length(false, true));
        System.out.println(identity(true, true));
        System.out.println(identity(false, true));
    }
}
"#,
    );
    fs::write(
        case.mappings.join("Program.map"),
        r#"
package readable;
class FirstLongPayload /* was raw/a */ {}
class SecondLongPayload /* was raw/b */ {}
class Probe /* was raw/Probe */ {}
"#,
    )
    .unwrap();
    let output = checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&case.original)
            .arg("-o")
            .arg(&case.generated)
            .arg("--mappings")
            .arg(&case.mappings)
            .args(["--no-api", "--threads", "1"]),
    );
    let source = case.source("Probe");
    contains(&source, "\"raw.a\"");
    contains(&source, "\"raw.b\"");
    contains(&source, "Class.forName($sf$className");
    contains(&source, "return \"readable.FirstLongPayload\";");
    contains(&source, "return \"readable.SecondLongPayload\";");
    contains(
        &String::from_utf8_lossy(&output.stderr),
        "also used as ordinary text",
    );
    case.roundtrip();
}

#[test]
fn shared_class_names_translate_at_original_parameter_boundaries_in_evaluation_order() {
    let case = Case::new(
        r#"
package raw;
class a {}
class b {
    static int events;
    static b receiver() { System.out.println(++events); return new b(); }
    static int after() { System.out.println(++events); return events; }
    Class a(String name, int after) throws Exception { return Class.forName(name); }
}
class c {
    Class type;
    c(String name) throws Exception { type = Class.forName(name); }
}
public class Probe {
    static void lookup(boolean array) throws Exception {
        String name = array ? "[Lraw.a;" : "raw.a";
        System.out.println(name);
        System.out.println(b.receiver().a(name, b.after()) == (array ? a[].class : a.class));
        System.out.println(name == (array ? "[Lraw.a;" : "raw.a"));
        System.out.println(Class.forName(name, false, Probe.class.getClassLoader()) == (array ? a[].class : a.class));
        System.out.println(new c(name).type == (array ? a[].class : a.class));
    }
    public static void main(String[] args) throws Exception { lookup(false); lookup(true); }
}
"#,
    );
    case.render(
        r#"
package readable;
class Payload /* was raw/a */ {}
class Resolver /* was raw/b */ {
    Class resolve(@ClassName String name, int after) /* was a */;
}
class Holder /* was raw/c */ { Holder(@ClassName String name) {} }
class Probe /* was raw/Probe */ {}
"#,
    );
    let source = case.source("Probe");
    contains(&source, ".resolve($sf$className");
    contains(&source, "new Holder($sf$className");
    contains(&source, "return \"[Lreadable.Payload;\";");
    case.roundtrip();
}

#[test]
fn shared_local_class_name_arrays_preserve_text_and_translate_lookup_arguments() {
    let case = Case::new(
        r#"
package raw;
class a {}
class b {}
public class Probe {
    static void lookup(boolean first) throws Exception {
        String[] names = new String[]{"raw.a", "raw.b"};
        Object holder = names;
        String[] alias = (String[])holder;
        alias[1] = "raw.b";
        System.out.println(names[0]);
        System.out.println(names[1]);
        System.out.println(Class.forName(alias[first ? 0 : 1]) == (first ? a.class : b.class));
        System.out.println(alias[0] == "raw.a");
        System.out.println(alias[1] == "raw.b");
    }
    public static void main(String[] args) throws Exception { lookup(true); lookup(false); }
}
"#,
    );
    case.render(
        r#"
package readable;
class FirstPayload /* was raw/a */ {}
class SecondPayload /* was raw/b */ {}
class Probe /* was raw/Probe */ {}
"#,
    );
    let source = case.source("Probe");
    contains(&source, "Class.forName($sf$className");
    contains(&source, "return \"readable.FirstPayload\";");
    contains(&source, "return \"readable.SecondPayload\";");
    case.roundtrip();
}

#[test]
fn annotated_array_publication_does_not_claim_complete_class_name_contents() {
    let case = Case::new(
        r#"
package raw;
class a {}
public class Probe {
    static void consume(String[] values) {}
    static void lookup() throws Exception {
        String[] names = new String[]{"raw.a"};
        consume(names);
        Class.forName(names[0]);
    }
}
"#,
    );
    let classes = crate::common::files(&case.original, "class")
        .into_iter()
        .map(|path| ClassFile::parse(&fs::read(path).unwrap()).unwrap())
        .collect::<Vec<_>>();
    let program = sporeflower::program::Program::new(&classes, &[]).unwrap();
    let mappings = sporeflower::mappings::parse_sources(
        &program,
        &[sporeflower::mappings::MappingSource::new(
            "Program.map",
            r#"
package raw;
class Probe /* was raw/Probe */ { void consume(@ClassName String[] values) /* was consume */; }
"#,
        )],
    )
    .unwrap();
    let member = program
        .resolve_method("raw/Probe", "lookup", "()V")
        .unwrap();
    let ir = sporeflower::ir::decompile_method(program.class(member.class), program.method(member))
        .unwrap();
    let analysis = sporeflower::semantics::Analysis::analyze(&ir, &program, &mappings, member);
    let input = analysis
        .class_name_consumers()
        .flat_map(|(_, inputs)| inputs)
        .find(|input| input.names.iter().any(|name| name == "raw.a"))
        .unwrap();
    assert!(!input.complete);
    assert!(!input.translate);
}

#[test]
fn shared_class_names_translate_when_stored_in_annotated_fields() {
    let case = Case::new(
        r#"
package raw;
class a {}
class b {}
public class Probe {
    static String saved;
    String instance;
    static Probe receiver(Probe value) { System.out.println("receiver"); return value; }
    static void lookup(boolean first) throws Exception {
        String shared = first ? "raw.a" : "raw.b";
        System.out.println(shared);
        saved = shared;
        Probe holder = new Probe();
        receiver(holder).instance = shared;
        System.out.println(Class.forName("raw.b") == b.class);
        System.out.println(Class.forName(saved) == (first ? a.class : b.class));
        System.out.println(Class.forName(holder.instance) == (first ? a.class : b.class));
        System.out.println(shared == (first ? "raw.a" : "raw.b"));
    }
    public static void main(String[] args) throws Exception { lookup(true); lookup(false); }
}
"#,
    );
    case.render(
        r#"
package readable;
class FirstPayload /* was raw/a */ {}
class SecondPayload /* was raw/b */ {}
class Probe /* was raw/Probe */ {
    @ClassName String saved /* was saved */;
    @ClassName String instance /* was instance */;
}
"#,
    );
    let source = case.source("Probe");
    contains(&source, "saved = $sf$className");
    contains(&source, ".instance = $sf$className");
    case.roundtrip();
}

#[test]
fn partial_class_name_candidates_do_not_translate_an_already_mapped_field_again() {
    let case = Case::new(
        r#"
package raw;
class a {}
class b {}
class Registry { static String type = "raw.b"; }
public class Probe {
    static boolean lookup(boolean local) throws Exception {
        String ordinary = "raw.a";
        System.out.println(ordinary);
        return Class.forName(local ? ordinary : Registry.type) == b.class;
    }
    public static void main(String[] args) throws Exception { System.out.println(lookup(false)); }
}
"#,
    );
    case.render(
        r#"
package raw;
class b /* was raw/a */ {}
class a /* was raw/b */ {}
class Registry /* was raw/Registry */ { @ClassName String type /* was type */; }
class Probe /* was raw/Probe */ {}
"#,
    );
    let source = fs::read_to_string(case.generated.join("raw/Probe.java")).unwrap();
    assert!(
        !source.contains("$sf$className"),
        "opaque field input was translated twice:\n{source}"
    );
    // This negative case checks the already-mapped field alternative. The
    // local alternative remains unsupported when both spellings coincide;
    // resolving that ambiguity needs interprocedural boundary provenance.
    crate::common::compile(&sources(&case.generated), &case.rebuilt, None);
    assert_eq!(
        execute(&case.original, "raw.Probe"),
        execute(&case.rebuilt, "raw.Probe")
    );
}

#[test]
fn mixed_class_name_alternatives_translate_once_through_rename_cycles() {
    let case = Case::new(
        r#"
package raw;
class a {}
class b {}
public class Probe {
    static boolean lookup(boolean first) throws Exception {
        String ordinary = "raw.a";
        System.out.println(ordinary);
        String name = first ? ordinary : "raw.b";
        return Class.forName(name) == (first ? a.class : b.class);
    }
    public static void main(String[] args) throws Exception {
        System.out.println(lookup(true));
        System.out.println(lookup(false));
    }
}
"#,
    );
    fs::write(
        case.mappings.join("Payloads.map"),
        r#"
package raw;
class b /* was raw/a */ {}
class a /* was raw/b */ {}
"#,
    )
    .unwrap();
    case.render("package raw; class Probe /* was raw/Probe */ {}");
    let source = fs::read_to_string(case.generated.join("raw/Probe.java")).unwrap();
    contains(&source, "Class.forName($sf$className");
    crate::common::compile(&sources(&case.generated), &case.rebuilt, None);
    assert_eq!(
        execute(&case.original, "raw.Probe"),
        execute(&case.rebuilt, "raw.Probe")
    );
}

#[test]
fn class_name_relocation_does_not_repair_invalid_slash_notation() {
    let case = Case::new(
        r#"
package raw;
class a {}
public class Probe {
    static boolean internalName() {
        try { Class.forName("raw/a"); return true; }
        catch (ClassNotFoundException expected) { return false; }
    }
    static boolean internalArrayName() {
        try { Class.forName("[Lraw/a;"); return true; }
        catch (ClassNotFoundException expected) { return false; }
    }
    static boolean sharedArrayName(boolean slash) {
        String name = slash ? "[Lraw/a;" : "[Lraw.a;";
        System.out.println(name);
        try { return Class.forName(name) == a[].class; }
        catch (ClassNotFoundException expected) { return false; }
    }
    public static void main(String[] args) {
        System.out.println(internalName());
        System.out.println(internalArrayName());
        System.out.println(sharedArrayName(false));
        System.out.println(sharedArrayName(true));
    }
}
"#,
    );
    case.render(
        r#"
package readable;
class Payload /* was raw/a */ {}
class Probe /* was raw/Probe */ {}
"#,
    );
    let source = case.source("Probe");
    contains(&source, "\"raw/a\"");
    contains(&source, "\"[Lraw/a;\"");
    case.roundtrip();
}

#[test]
fn opaque_record_and_plane_alternatives_suppress_only_their_layout_names() {
    let case = Case::new(
        r#"
package raw;
public class Probe {
    static int[] records = new int[]{11, 22, 33, 44};
    static int[] planes = new int[]{1, 2, 3, 4, 11, 12, 13, 14};
    static int[] opaqueRecord() { return new int[]{1, 2, 3, 4}; }
    static int[] opaquePlane() { return new int[]{5, 6, 7, 8, 15, 16, 17, 18}; }
    static int ambiguousRecord(boolean known, int row) {
        int[] data = known ? records : opaqueRecord();
        return data[row * 2 + 1];
    }
    static int ambiguousPlane(boolean known, int index) {
        int[] data = known ? planes : opaquePlane();
        return data[(index & 3) + 4];
    }
    static int knownRecord(int row) { return records[row * 2 + 1]; }
    static int knownPlane(int index) { return planes[(index & 3) + 4]; }
    public static void main(String[] args) {
        System.out.println(ambiguousRecord(true, 1));
        System.out.println(ambiguousRecord(false, 1));
        System.out.println(ambiguousPlane(true, 2));
        System.out.println(ambiguousPlane(false, 2));
        System.out.println(knownRecord(1));
        System.out.println(knownPlane(2));
    }
}
"#,
    );
    case.render(
        r#"
package readable;
@SlotDomain interface Slot { int KEY = 0; int STATE = 1; }
class Probe /* was raw/Probe */ {
    @Records(value = Slot.class, stride = 2) int[] records /* was records */;
    @Planes(value = Slot.class, stride = 4) int[] planes /* was planes */;
}
"#,
    );
    let source = case.source("Probe");
    let method = |name: &str| {
        source
            .split(&format!("static int {name}("))
            .nth(1)
            .unwrap()
            .split("\n    }")
            .next()
            .unwrap()
    };
    for name in ["ambiguousRecord", "ambiguousPlane"] {
        assert!(
            !method(name).contains("Slot."),
            "opaque alternative must suppress {name}'s layout names:\n{source}"
        );
    }
    contains(method("knownRecord"), "Slot.STATE");
    contains(method("knownPlane"), "Slot.STATE");
    case.roundtrip();
}

#[test]
fn array_index_meaning_requires_every_alternative_to_have_that_index_contract() {
    let case = Case::new(
        r#"
package raw;
public class Probe {
    static int[] indexed = new int[]{10, 20};
    static int[] valued = new int[]{1, 2};
    static int ambiguous(boolean useIndexed) {
        int[] data = useIndexed ? indexed : valued;
        return data[1];
    }
    static int known() { return indexed[1]; }
    public static void main(String[] args) {
        System.out.println(ambiguous(true));
        System.out.println(ambiguous(false));
        System.out.println(known());
    }
}
"#,
    );
    case.render(
        r#"
package readable;
@ValueDomain interface Position { int FIRST = 0; int SECOND = 1; }
@ValueDomain interface Value { int OFF = 1; int ON = 2; }
class Probe /* was raw/Probe */ {
    @IndexDomain(value = Position.class, dimension = 0) int[] indexed /* was indexed */;
    @Domain(Value.class) int[] valued /* was valued */;
}
"#,
    );
    let source = case.source("Probe");
    let ambiguous = source
        .split("static int ambiguous(")
        .nth(1)
        .unwrap()
        .split("\n    }")
        .next()
        .unwrap();
    assert!(
        !ambiguous.contains("Position."),
        "a leaf contract on another array cannot establish its indexes:\n{source}"
    );
    contains(&source, "indexed[Position.SECOND]");
    case.roundtrip();
}

#[test]
fn selector_variants_share_packing_constants_without_merging_their_value_domains() {
    let case = Case::new(
        r#"
package raw;
class a {
    static int a(int kind, int subtype) { return kind | subtype << 1; }
    static int b(int kind, int subtype) { return kind | (subtype & 15) << 1; }
    static int c(int word) {
        int subtype = word >>> 1 & 15;
        if ((word & 1) == 0) return subtype == 2 ? 11 : 12;
        return subtype == 2 ? 21 : 22;
    }
}
public class Probe {
    public static void main(String[] args) {
        int[] values = {-1, 0, 2, 7, 2147483647};
        for (int i = 0; i < values.length; i++) {
            for (int kind = 0; kind < 2; kind++) {
                System.out.println(a.a(kind, values[i]));
                System.out.println(a.b(kind, values[i]));
                System.out.println(a.c(a.b(kind, values[i])));
            }
        }
    }
}
"#,
    );
    case.render(
        r#"
package readable;
@ValueDomain interface Family { int FRUIT = 0; int SHAPE = 1; }
@ValueDomain interface Fruit { int BERRY = 2; }
@ValueDomain interface Shape { int SQUARE = 2; }
@PackedDomain
@BitField(name = "kind", value = Family.class, bits = 1)
@BitField(name = "subtype", value = Fruit.class, shift = 1, bits = 4, selectorMask = 1, selectorValue = 0)
@BitField(name = "subtype", value = Shape.class, shift = 1, bits = 4, selectorMask = 1, selectorValue = 1)
interface TaggedWord {}
class Packing /* was raw/a */ {
    @Domain(TaggedWord.class) int pack(@Domain(Family.class) int kind, int subtype) /* was a */;
    @Domain(TaggedWord.class) int packMasked(@Domain(Family.class) int kind, int subtype) /* was b */;
    int read(@Domain(TaggedWord.class) int word) /* was c */;
}
class Probe /* was raw/Probe */ {}
"#,
    );
    let source = case.source("Packing");
    contains(&source, "subtype << TaggedWord.SUBTYPE_SHIFT");
    contains(
        &source,
        "(subtype & TaggedWord.SUBTYPE_VALUE_MASK) << TaggedWord.SUBTYPE_SHIFT",
    );
    contains(&source, "int subtype =");
    contains(&source, "Family.FRUIT");
    contains(&source, "Fruit.BERRY");
    contains(&source, "Shape.SQUARE");
    case.roundtrip();
}

#[test]
fn packed_clear_masks_preserve_int_long_widths_and_do_not_name_other_bits() {
    let case = Case::new(
        r#"
package raw;
class a {
    static int a(int word) { return word & -256; }
    static long a(long word) { return word & -256L; }
    static long b(long word) { return word & 4294967040L; }
    static int b(int word) { return word & 2147483647; }
    static long c(long word) { return word & 9223372036854775807L; }
    static long d(long word) { return word & -4294967296L; }
    static int c(int word) { return word & 0; }
    static byte d(int word) { return (byte)(word & -256); }
    static long e(long word) { return word & -71776119061217281L; }
    static int e(int word) { return word & -31; }
}
public class Probe {
    public static void main(String[] args) {
        int[] ints = {-1, 0, 305419896, -2147483648, 2147483647};
        for (int i = 0; i < ints.length; i++) {
            System.out.println(a.a(ints[i]));
            System.out.println(a.b(ints[i]));
            System.out.println(a.c(ints[i]));
            System.out.println(a.d(ints[i]));
            System.out.println(a.e(ints[i]));
        }
        long[] longs = {-1L, 0L, 81985529216486895L, -9223372036854775808L, 9223372036854775807L};
        for (int i = 0; i < longs.length; i++) {
            System.out.println(a.a(longs[i]));
            System.out.println(a.b(longs[i]));
            System.out.println(a.c(longs[i]));
            System.out.println(a.d(longs[i]));
            System.out.println(a.e(longs[i]));
        }
    }
}
"#,
    );
    case.render(
        r#"
package readable;
@PackedDomain @BitField(name = "low", bits = 8) interface LowByte {}
@PackedDomain @BitField(name = "high", shift = 31, bits = 1) interface IntHigh {}
@PackedDomain @BitField(name = "high", shift = 63, bits = 1) interface LongHigh {}
@PackedDomain @BitField(name = "word", bits = 32) interface LowWord {}
@PackedDomain @BitField(name = "generation", shift = 48, bits = 7) interface SearchTag {}
@PackedDomain
@BitField(name = "first", shift = 1, bits = 4, selectorMask = 1, selectorValue = 0)
@BitField(name = "second", shift = 1, bits = 4, selectorMask = 1, selectorValue = 1)
interface AlternateNames {}
class Clearing /* was raw/a */ {
    int clearInt(@Domain(LowByte.class) int word) /* was a */;
    long clearLong(@Domain(LowByte.class) long word) /* was a */;
    long keepLowWord(@Domain(LowByte.class) long word) /* was b */;
    int clearIntSign(@Domain(IntHigh.class) int word) /* was b */;
    long clearLongSign(@Domain(LongHigh.class) long word) /* was c */;
    long clearLowWord(@Domain(LowWord.class) long word) /* was d */;
    int clearWholeInt(@Domain(LowWord.class) int word) /* was c */;
    byte narrowCleared(@Domain(LowByte.class) int word) /* was d */;
    long clearEightTagBits(@Domain(SearchTag.class) long word) /* was e */;
    int ambiguousName(@Domain(AlternateNames.class) int word) /* was e */;
}
class Probe /* was raw/Probe */ {}
"#,
    );
    let source = case.source("Clearing");
    contains(&source, "word & ~LowByte.LOW_MASK");
    contains(&source, "word & ~(long) LowByte.LOW_MASK");
    contains(&source, "word & ~(int) IntHigh.HIGH_MASK");
    contains(&source, "word & ~LongHigh.HIGH_MASK");
    contains(&source, "word & ~LowWord.WORD_MASK");
    contains(&source, "word & ~(int) LowWord.WORD_MASK");
    contains(&source, "word & 4294967040L");
    contains(&source, "word & -71776119061217281L");
    contains(&source, "word & -31");
    case.roundtrip();
}

#[test]
fn shadowed_constant_owners_keep_literals_and_authored_parameters() {
    let case = Case::new(
        r#"
package raw;
class a {
    static int a(int code, int x, int y) {
        switch (code) {
            case 1: return x + y;
            default: return -1;
        }
    }
    static byte b(int x, int y) { return 1; }
}
public class Probe {
    public static void main(String[] args) {
        System.out.println(a.a(1, 5, 7));
        System.out.println(a.a(2, 5, 7));
        System.out.println(a.b(5, 7));
    }
}
"#,
    );
    case.render(
        r#"
package readable;
@ValueDomain interface State { int ACTIVE = 1; }
class Reader /* was raw/a */ {
    int choose(@Domain(State.class) int code, int State, int readable) /* was a */;
    @Domain(State.class) byte narrow(int State, int readable) /* was b */;
}
class Probe /* was raw/Probe */ {}
"#,
    );
    let source = case.source("Reader");
    contains(&source, "choose(int code, int State, int readable)");
    contains(&source, "narrow(int State, int readable)");
    contains(&source, "case 1:");
    assert!(!source.contains("State.ACTIVE"), "{source}");
    case.roundtrip();
}

#[test]
fn omitted_default_keys_preserve_only_distinct_key_mapping_constraints() {
    let case = Case::new(
        r#"
package raw;
class a {
    static int a(int code) {
        switch (code) {
            case 0: return b(code, 2);
            case 1:
            case 3:
            default: return b(code, 2);
            case 5: return b(code, 2) + 1;
        }
    }
    static int b(int code, int payload) { return code * 100 + payload; }
}
public class Probe {
    public static void main(String[] args) {
        for (int code = -2; code <= 7; code++) {
            System.out.println(a.a(code));
        }
    }
}
"#,
    );
    case.render(
        r#"
package readable;
@ValueDomain interface Mode { int ZERO = 0; int ONE = 1; int THREE = 3; int FIVE = 5; }
@ValueDomain interface ZeroValue { int READY = 2; }
@ValueDomain interface OneValue { int FINISHED = 2; }
class Reader /* was raw/a */ {
    int choose(@Domain(Mode.class) int code) /* was a */;
    int consume(@Domain(Mode.class) int code,
        @DomainWhen(value = ZeroValue.class, parameter = 0, equals = 0)
        @DomainWhen(value = OneValue.class, parameter = 0, equals = 1)
        int payload) /* was b */;
}
class Probe /* was raw/Probe */ {}
"#,
    );
    let source = case.source("Reader");
    let choose = crate::common::source::method(&source, "choose");
    contains(choose, "case Mode.ZERO:");
    contains(choose, "case Mode.FIVE:");
    contains(choose, "consume(code, ZeroValue.READY)");
    contains(choose, "consume(code, 2)");
    assert_eq!(choose.matches("case ").count(), 2, "{choose}");
    assert!(!choose.contains("OneValue.FINISHED"), "{choose}");
    case.roundtrip();
}

#[test]
fn secondary_record_cursors_name_columns_only_with_proven_residues() {
    let case = Case::new(
        r#"
package raw;
class a {
    static int[] a = new int[32];
    static int[] b = new int[12];
    static int a(int count) {
        int base = 0;
        int total = 0;
        for (int i = 0; i < count; i++) {
            int stateIndex = base + 1;
            int countIndex = base + 2;
            a[stateIndex] = 2;
            a[countIndex] += 1;
            total += a[stateIndex] + a[countIndex];
            base += 8;
        }
        return total;
    }
    static int b(int count, int base) {
        int total = 0;
        for (int i = 0; i < count; i++) {
            total += a[base + 1];
            base += 8;
        }
        return total;
    }
    static int c(int count) {
        int base = 0;
        int total = 0;
        for (int i = 0; i < count; i++) {
            b[base + 1] = 2;
            total += b[base + 1];
            base += 3;
        }
        return total;
    }
}
public class Probe {
    public static void main(String[] args) {
        System.out.println(a.a(3));
        System.out.println(a.a(3));
        System.out.println(a.b(3, 0));
        System.out.println(a.b(3, 1));
        System.out.println(a.c(3));
    }
}
"#,
    );
    case.render(
        r#"
package readable;
@ValueDomain interface State { int READY = 2; }
@SlotDomain interface EntrySlot {
    @SlotValue(State.class) int STATE = 1;
    int COUNT = 2;
}
@SlotDomain interface TripletSlot { int VALUE = 1; }
class Cursors /* was raw/a */ {
    @Records(value = EntrySlot.class, stride = 8) int[] entries /* was a */;
    @Records(value = TripletSlot.class, stride = 3) int[] triples /* was b */;
    int aligned(int count) /* was a */;
    int opaque(int count, int base) /* was b */;
    int nonPowerOfTwo(int count) /* was c */;
}
class Probe /* was raw/Probe */ {}
"#,
    );
    let source = case.source("Cursors");
    contains(&source, "+ EntrySlot.STATE");
    contains(&source, "+ EntrySlot.COUNT");
    contains(&source, "= State.READY;");
    let opaque = source
        .split("int opaque(")
        .nth(1)
        .unwrap()
        .split("\n    }")
        .next()
        .unwrap();
    assert!(!opaque.contains("EntrySlot."), "{source}");
    assert!(!source.contains("TripletSlot.VALUE"), "{source}");
    case.roundtrip();
}

#[test]
fn contextual_contracts_cannot_replace_producer_packed_layouts() {
    let case = Case::new(
        r#"
package raw;
public class Probe {
    static void accept(int word) { System.out.println(word); }
    static boolean known(boolean choose, int word) {
        int selected = choose ? word : 1;
        accept(selected);
        return (selected & 3) == 1;
    }
    static boolean opaque(int word) {
        accept(word);
        return (word & 3) == 1;
    }
    public static void main(String[] args) {
        System.out.println(known(true, 5));
        System.out.println(known(false, 6));
        System.out.println(opaque(5));
        System.out.println(opaque(6));
    }
}
"#,
    );
    case.render(
        r#"
package readable;
@ValueDomain interface Kind { int FIRST = 1; }
@ValueDomain interface OtherKind { int SECOND = 1; }
@PackedDomain @BitField(name="kind", value=Kind.class, bits=2) interface Produced {}
@PackedDomain @BitField(name="other", value=OtherKind.class, bits=2) interface Consumed {}
class Probe /* was raw/Probe */ {
    void accept(@Domain(Consumed.class) int word) /* was accept */;
    boolean known(boolean choose, @Domain(Produced.class) int word) /* was known */;
    boolean opaque(int word) /* was opaque */;
}
"#,
    );
    let source = case.source("Probe");
    let method = |name: &str| {
        source
            .split(&format!("static boolean {name}("))
            .nth(1)
            .unwrap()
            .split("\n    }")
            .next()
            .unwrap()
    };
    contains(method("known"), "Produced.KIND_MASK");
    contains(method("known"), "Kind.FIRST");
    assert!(!method("known").contains("Consumed."), "{source}");
    assert!(!method("opaque").contains(".OTHER_"), "{source}");
    assert!(!method("opaque").contains("OtherKind."), "{source}");
    case.roundtrip();
}
