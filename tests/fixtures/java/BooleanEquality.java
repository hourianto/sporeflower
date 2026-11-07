// The test changes (II)I descriptors to (ZZ)I after compilation. JVM slots
// still carry arbitrary ints, which direct boolean equality must preserve.
public class BooleanEquality {
    public static int equal(int left, int right) { return left == right ? 1 : 0; }
    public static int different(int left, int right) { return left != right ? 1 : 0; }
}
