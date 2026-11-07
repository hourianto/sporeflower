//! Independently authored literals exercise presentation, Java typing, and bits.
use crate::common::source::method;
use crate::common::{checked, compile, execute, sources};
use std::{fs, process::Command};

#[test]
fn exact_standard_constants_preserve_context_overloads_and_runtime_bits() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("StandardConstants.java");
    fs::write(&input, r#"
public class StandardConstants {
    static final int LOW = -2147483648;
    static final long HIGH = 9223372036854775807L;
    static final byte BYTE = -128;
    static final short SHORT = 32767;
    static final double CIRCLE = 3.141592653589793;
    static final float TINY = 1.4e-45F;
    static int integer(boolean high) { return high ? 2147483647 : -2147483648; }
    static long wide(boolean high) { return high ? 9223372036854775807L : -9223372036854775808L; }
    static byte narrow(boolean high) { return high ? (byte) 127 : (byte) -128; }
    static short small(boolean high) { return high ? (short) 32767 : (short) -32768; }
    static double pi() { return 3.141592653589793; }
    static double e() { return 2.718281828459045; }
    static double nearby() { return 3.1415926535897936; }
    static float floatPi() { return 3.1415927F; }
    static int ordinary() { return 32767; }
    static long wideInt() { return 2147483647L; }
    static int overflow(int value) { return value + -2147483648; }
    static long wideOverflow(long value) { return value - -9223372036854775808L; }
    static int selected(int value) {
        switch (value) {
            case -2147483648: return 1;
            case 2147483647: return 2;
            default: return 3;
        }
    }
    static int overload(byte value) { return 1; }
    static int overload(short value) { return 2; }
    static int overload(int value) { return 3; }
    static int overload(long value) { return 4; }
    static int overload(float value) { return 5; }
    static int overload(double value) { return 6; }
    static int calls() {
        return overload((byte) 127) + overload((short) -32768) + overload(127)
            + overload(32767) + overload(2147483647) + overload(2147483647L)
            + overload(3.1415927F) + overload(3.141592653589793);
    }
    static float[] floats() {
        return new float[] { 1.4e-45F, 3.4028235e38F, 1.0F / 0.0F, -1.0F / 0.0F,
            0.0F / 0.0F, 0.0F, -0.0F, 2.8e-45F, 3.1415927F };
    }
    static double[] doubles() {
        return new double[] { 4.9e-324, 1.7976931348623157e308, 1.0D / 0.0D, -1.0D / 0.0D,
            0.0D / 0.0D, 0.0D, -0.0D, 1.0e-323, 3.141592653589793, 2.718281828459045 };
    }
    public static void main(String[] args) {
        for (int i = 0; i < 2; i++) {
            boolean high = i != 0;
            System.out.println(integer(high) + ":" + wide(high) + ":" + narrow(high) + ":" + small(high));
            System.out.println(overflow(integer(high)) + ":" + wideOverflow(wide(high)) + ":" + selected(integer(high)));
        }
        System.out.println(calls() + ":" + ordinary() + ":" + wideInt());
        System.out.println(pi() + ":" + e() + ":" + nearby() + ":" + floatPi());
        float[] single = floats();
        for (int i = 0; i < single.length; i++) System.out.println(Float.floatToIntBits(single[i]));
        double[] dual = doubles();
        for (int i = 0; i < dual.length; i++) System.out.println(Double.doubleToLongBits(dual[i]));
    }
}
"#).unwrap();
    let original = work.path().join("original");
    compile(&[input], &original, None);
    let generated = work.path().join("generated");
    let api = work.path().join("api.jar");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&generated)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    // javac folds these field names away. The trimmed API still needs their
    // declarations when the decompiler introduces them again, even without an
    // optional legacy compiler installed on the test machine.
    let declarations = sporeflower::input::Input::read(&api)
        .unwrap()
        .into_classes();
    for (owner, descriptor, fields) in [
        ("Byte", "B", &["MIN_VALUE", "MAX_VALUE"][..]),
        ("Short", "S", &["MIN_VALUE", "MAX_VALUE"][..]),
        ("Integer", "I", &["MIN_VALUE", "MAX_VALUE"][..]),
        ("Long", "J", &["MIN_VALUE", "MAX_VALUE"][..]),
        (
            "Float",
            "F",
            &[
                "MIN_VALUE",
                "MAX_VALUE",
                "NaN",
                "POSITIVE_INFINITY",
                "NEGATIVE_INFINITY",
            ][..],
        ),
        (
            "Double",
            "D",
            &[
                "MIN_VALUE",
                "MAX_VALUE",
                "NaN",
                "POSITIVE_INFINITY",
                "NEGATIVE_INFINITY",
            ][..],
        ),
        ("Math", "D", &["PI", "E"][..]),
    ] {
        let owner = format!("java/lang/{owner}");
        let declaration = declarations
            .iter()
            .find(|class| class.name == owner)
            .unwrap_or_else(|| panic!("standard constant owner absent from exported API: {owner}"));
        for name in fields {
            assert!(
                declaration.fields.iter().any(|field| field.name == *name
                    && field.descriptor == descriptor
                    && field.access_flags & 0x19 == 0x19
                    && field.constant_value.is_some()),
                "missing constant {owner}.{name}:{descriptor}"
            );
        }
    }
    let text = fs::read_to_string(generated.join("StandardConstants.java")).unwrap();
    for spelling in [
        "int LOW = Integer.MIN_VALUE",
        "long HIGH = Long.MAX_VALUE",
        "byte BYTE = Byte.MIN_VALUE",
        "short SHORT = Short.MAX_VALUE",
        "double CIRCLE = Math.PI",
        "float TINY = Float.MIN_VALUE",
        "Byte.MAX_VALUE",
        "Short.MIN_VALUE",
        "Math.E",
        "Float.MAX_VALUE",
        "Float.POSITIVE_INFINITY",
        "Float.NEGATIVE_INFINITY",
        "Float.NaN",
        "Double.MIN_VALUE",
        "Double.MAX_VALUE",
        "Double.POSITIVE_INFINITY",
        "Double.NEGATIVE_INFINITY",
        "Double.NaN",
        "case Integer.MIN_VALUE:",
        "case Integer.MAX_VALUE:",
    ] {
        assert!(text.contains(spelling), "missing {spelling}:\n{text}");
    }
    assert!(
        method(&text, "ordinary").contains("return 32767;"),
        "{text}"
    );
    assert!(method(&text, "wideInt").contains("2147483647L"), "{text}");
    for name in ["nearby", "floatPi"] {
        assert!(!method(&text, name).contains("Math."), "{text}");
    }
    assert!(
        method(&text, "overflow").contains("+ Integer.MIN_VALUE"),
        "{text}"
    );
    assert!(
        method(&text, "wideOverflow").contains("- Long.MIN_VALUE"),
        "{text}"
    );
    let rebuilt = work.path().join("rebuilt");
    let files = sources(&generated);
    compile(&files, &rebuilt, None);
    assert_eq!(
        execute(&original, "StandardConstants"),
        execute(&rebuilt, "StandardConstants")
    );
    let legacy = work.path().join("legacy");
    if crate::common::legacy_compile(&files, &legacy, &api) {
        assert_eq!(
            execute(&original, "StandardConstants"),
            execute(&legacy, "StandardConstants")
        );
    }
}

#[test]
fn standard_constant_qualifiers_respect_types_fields_and_local_names() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("Shadows.java");
    fs::write(&input, r#"
package collisions;
class Integer {}
class Math {}
class Fields {
    static Object Float;
    static int integer() { return 2147483647; }
    static double pi() { return 3.141592653589793; }
    static float infinity() { return 1.0F / 0.0F; }
    static int local(int Integer) { return Integer + 2147483647; }
}

class Blocked {
    static Object Float;
    static final int LIMIT = 2147483647;
    static int integer(int java) { return -2147483648; }
    static double pi() { return 3.141592653589793; }
    static float infinity() { return 1.0F / 0.0F; }
}
public class Shadows {
    public static void main(String[] args) {
        System.out.println(Fields.integer() + ":" + Fields.pi() + ":" + Fields.infinity() + ":" + Fields.local(3));
        System.out.println(Blocked.integer(0) + ":" + Blocked.pi() + ":" + Blocked.infinity());
    }
}
"#).unwrap();
    let original = work.path().join("original");
    fs::create_dir_all(&original).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-g", "-d"])
            .arg(&original)
            .arg(&input),
    );
    let generated = work.path().join("generated");
    let mappings = work.path().join("Shadows.map");
    fs::write(
        &mappings,
        "package collisions; class Blocked /* was collisions/Blocked */ { int integer(int java) /* was integer */; }",
    )
    .unwrap();
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&generated)
            .arg("--mappings")
            .arg(&mappings)
            .args(["--no-api", "--threads", "1"]),
    );
    let fields = fs::read_to_string(generated.join("collisions/Fields.java")).unwrap();
    for spelling in [
        "java.lang.Integer.MAX_VALUE",
        "java.lang.Math.PI",
        "java.lang.Float.POSITIVE_INFINITY",
    ] {
        assert!(fields.contains(spelling), "missing {spelling}:\n{fields}");
    }
    let blocked = fs::read_to_string(generated.join("collisions/Blocked.java")).unwrap();
    assert!(blocked.contains("int LIMIT = 2147483647"), "{blocked}");
    assert!(blocked.contains("return -2147483648;"), "{blocked}");
    assert!(!blocked.contains("java.lang."), "{blocked}");
    let rebuilt = work.path().join("rebuilt");
    compile(&sources(&generated), &rebuilt, None);
    assert_eq!(
        execute(&original, "collisions.Shadows"),
        execute(&rebuilt, "collisions.Shadows")
    );
}

#[test]
fn standard_constants_respect_inherited_library_member_types() {
    let work = tempfile::tempdir().unwrap();
    let base = work.path().join("Base.java");
    let root = work.path().join("Root.java");
    fs::write(
        &base,
        "package lib; public class Base { public static class Integer {} protected static class Math {} }",
    )
    .unwrap();
    fs::write(
        &root,
        "package lib; public class Root extends Base { public static class java {} }",
    )
    .unwrap();
    let library = work.path().join("library");
    compile(&[base, root], &library, None);
    let input = work.path().join("Probe.java");
    fs::write(
        &input,
        r#"
package application;
public class Probe extends lib.Base {
    static int limit() { return 2147483647; }
    static double circle() { return 3.141592653589793; }
}
class Blocked extends lib.Root {
    static int limit() { return 2147483647; }
    static double circle() { return 3.141592653589793; }
}
"#,
    )
    .unwrap();
    let original = work.path().join("original");
    compile(&[input], &original, Some(&library));
    let generated = work.path().join("generated");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&generated)
            .arg("--classpath")
            .arg(&library)
            .args(["--no-api", "--threads", "1"]),
    );
    let probe = fs::read_to_string(generated.join("application/Probe.java")).unwrap();
    assert!(probe.contains("java.lang.Integer.MAX_VALUE"), "{probe}");
    assert!(probe.contains("java.lang.Math.PI"), "{probe}");
    let blocked = fs::read_to_string(generated.join("application/Blocked.java")).unwrap();
    assert!(blocked.contains("return 2147483647;"), "{blocked}");
    assert!(blocked.contains("return 3.141592653589793D;"), "{blocked}");
    compile(
        &sources(&generated),
        &work.path().join("rebuilt"),
        Some(&library),
    );
}
