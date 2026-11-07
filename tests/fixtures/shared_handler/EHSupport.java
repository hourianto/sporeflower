public final class EHSupport {
    public static String trace;
    public static final Object monitor = new Object();
    public static final RuntimeException[] failures = new RuntimeException[16];
    static {
        for (int i = 0; i < failures.length; ++i) {
            failures[i] = (i & 1) == 0 ? new IllegalStateException("E" + i) : new IllegalArgumentException("E" + i);
        }
    }
    public static int step(int code, int mask) {
        trace += code + ",";
        if ((mask & (1 << code)) != 0) throw failures[code];
        return code;
    }
    public static boolean condition(int code, int mask, boolean value) {
        step(code, mask);
        return value;
    }
    public static int identity(Throwable value) {
        for (int i = 0; i < failures.length; ++i) if (value == failures[i]) return i;
        return value == null ? -1 : -2;
    }
}
