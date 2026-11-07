package pkg;
public class ExceptionDispatchState {
    public static int cleanups, catches, tails;
    public static boolean failCleanup;
    public static Throwable escaped;
    public static int body(int mode) {
        if (mode == 1) { IllegalArgumentException x = new IllegalArgumentException("body"); escaped = x; throw x; }
        if (mode == 2) { AssertionError x = new AssertionError("body"); escaped = x; throw x; }
        return 9;
    }
    public static void cleanup() {
        cleanups++;
        if (failCleanup) { IllegalStateException x = new IllegalStateException("cleanup"); escaped = x; throw x; }
    }
    public static void caught(int mode) {
        catches++;
        if (mode == 3) throw new UnsupportedOperationException("catch");
    }
    public static int finish(int mode, int value) {
        if (mode == 3) throw new UnsupportedOperationException("second body");
        return value + 7;
    }
    public static void tail() { tails++; }
    public static void unreachable() { throw new AssertionError("shadowed handler"); }
}
