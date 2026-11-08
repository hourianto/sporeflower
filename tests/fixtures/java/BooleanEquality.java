// The test changes (II)I descriptors to (ZZ)I after compilation. JVM slots
// still carry arbitrary ints, which direct boolean equality must preserve.
public class BooleanEquality {
    public static int equal(int left, int right) { return left == right ? 1 : 0; }
    public static int different(int left, int right) { return left != right ? 1 : 0; }
    public static int exactlyTrue(int raw, int unused) { return raw == 1 ? 1 : 0; }
    public static int notExactlyTrue(int raw, int unused) { return raw != 1 ? 1 : 0; }
    public static int reversedTrue(int raw, int unused) { return 1 == raw ? 1 : 0; }
    public static int normalizedTrue(int raw, int unused) { return (raw != 0 ? 1 : 0) == 1 ? 1 : 0; }
    public static int normalizedFalse(int raw, int unused) { return (raw != 0 ? 1 : 0) != 1 ? 1 : 0; }
}
