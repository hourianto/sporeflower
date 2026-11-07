public class SparseProtection {
    private static int trace;
    private static int rounds;
    private static final IllegalArgumentException marker = new IllegalArgumentException();
    private static boolean next() {
        rounds = rounds + 1;
        return rounds < 3;
    }
    private static void step(int code, int fail) {
        trace = trace * 31 + code;
        if (code == fail) throw marker;
    }
    public static int run(int mode) {
        trace = 0;
        try {
            step(1, mode);
            if ((mode & 1) == 0) step(2, mode);
            else step(3, mode);
            for (int i = 0; i < 3; i++) {
                if ((i & 1) == 0) step(4, mode);
                else step(5, mode);
            }
            return trace;
        } catch (IllegalArgumentException failure) {
            return -trace;
        } catch (RuntimeException failure) {
            return 123456;
        }
    }
    public static int copiedLoops(int mode) {
        try {
            if (mode == 0) step(1, 1);
            else step(2, 2);
        } catch (IllegalArgumentException failure) {
            if (failure != marker) throw new IllegalStateException("identity");
            rounds = 0;
            outer: while (next()) {
                for (int i = 0; i < 3; i++) {
                    if (i == 1) {
                        continue outer;
                    }
                    trace = trace * 31 + i + 1;
                }
            }
        }
        return trace;
    }
    private static void checked() throws java.io.IOException { }
    private static void checkedThrow(int mode) throws java.io.IOException {
        if (mode != 0) throw new java.io.IOException();
    }
    public static int monitored(int mode) {
        try {
            synchronized (marker) {
                checkedThrow(mode);
                return 3;
            }
        } catch (java.io.IOException failure) { return 4; }
    }
    public static int impossible(int[] first, int[] second, boolean choose) {
        try {
            if (choose) {
                int length = first.length;
                checked();
                return length;
            } else {
                int length = second.length;
                checked();
                return length;
            }
        } catch (java.io.IOException failure) { return 123456; }
    }
    public static void main(String[] args) {
        for (int mode = 0; mode < 7; mode++) System.out.println(run(mode));
        System.out.println(copiedLoops(0));
        System.out.println(copiedLoops(1));
        System.out.println(monitored(0));
        System.out.println(monitored(1));
        System.out.println(Thread.holdsLock(marker));
        for (int mode = 0; mode < 4; mode++) {
            try {
                System.out.println(impossible(mode == 2 ? null : new int[2], mode == 3 ? null : new int[3], (mode & 1) == 0));
            } catch (Throwable failure) {
                System.out.println(failure.getClass().getName());
            }
        }
    }
}
