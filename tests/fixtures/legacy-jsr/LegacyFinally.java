/** Compiled with legacy javac to retain real jsr/ret finally subroutines. */
public class LegacyFinally {
    private static int trace;

    private static int value(int n) {
        trace = trace * 10 + 1;
        if (n < 0) throw new IllegalArgumentException();
        return n * 3;
    }

    public static int run(int n) {
        try {
            if (n == 0) return 11;
            return value(n);
        } catch (IllegalArgumentException e) {
            return -7;
        } finally {
            trace = trace * 10 + 2;
        }
    }

    public static int nested(int count) {
        int result = 0;
        outer: for (int i = 0; i < count; i++) {
            try {
                try {
                    if (i == 1) continue outer;
                    if (i == 3) break outer;
                    result += value(i);
                } finally {
                    trace = trace * 10 + 3;
                }
            } finally {
                trace = trace * 10 + 4;
            }
        }
        return result;
    }

    public static int abandoned(int mode) {
        int result = 0;
        outer: for (int i = 0; i < 3; i++) {
            try {
                result += 10;
                if (mode == 0) return result;
                if (mode == 1) throw new IllegalArgumentException();
            } finally {
                trace = trace * 10 + 5;
                if (i == 0) continue outer;
                if (mode == 2) break outer;
                if (mode == 3) return result + 7;
            }
            result++;
        }
        return result;
    }

    static int protectedCalls(int n) {
        try { trace++; } finally { trace += 2; }
        try {
            try {
                value(n);
                if (n > 2) value(n + 1);
                value(n + 2);
                if (n > 3) value(n + 3);
                value(n + 4);
            } catch (IllegalArgumentException e) {
                trace = 1;
            }
            trace++;
        } catch (RuntimeException e) {
            return -1;
        }
        return trace;
    }

    static int protectedLoop(int count, int[] slots) {
        try { trace++; } finally { trace += 2; }
        try {
            for (int i = 0; i < count; i++) {
                slots[i & 3] = i;
                try {
                    value(i - 1);
                    if (i > 2) value(i + 1);
                    value(i + 2);
                    if (i > 3) value(i + 3);
                    value(i + 4);
                    if (i > 4) value(i + 5);
                    value(i + 6);
                } catch (IllegalArgumentException e) {
                    trace = 1;
                }
                trace++;
            }
        } catch (RuntimeException e) {
            return -1;
        }
        return trace;
    }

    public static void main(String[] args) {
        for (int i = -2; i < 3; i++) {
            trace = 0;
            System.out.println(run(i));
            System.out.println(protectedCalls(i));
            System.out.println(trace);
        }
        trace = 0;
        System.out.println(protectedLoop(6, new int[4]));
        System.out.println(protectedLoop(1, null));
        System.out.println(nested(5));
        System.out.println(trace);
        for (int mode = 0; mode < 5; mode++) {
            trace = 0;
            try { System.out.println(abandoned(mode)); }
            catch (IllegalArgumentException expected) { System.out.println("caught"); }
            System.out.println(trace);
        }
    }
}
