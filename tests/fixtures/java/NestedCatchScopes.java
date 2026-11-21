public class NestedCatchScopes {
    static int trace;
    private static final Object lock = new Object();

    static void step(int current, int failure) {
        trace = trace * 10 + current;
        if (current == failure) throw new IllegalStateException();
    }

    static int run(int failure) {
        try {
            step(1, failure);
            try {
                step(2, failure);
            } catch (Exception first) {
                step(3, failure);
            }
            step(4, failure);
            try {
                step(5, failure);
            } catch (Exception second) {
                step(6, failure);
            }
            return trace;
        } catch (Exception outer) {
            return -trace;
        }
    }

    static int quiet(int failure) {
        synchronized (lock) {
            try {
                step(1, failure);
                try {
                    step(2, failure);
                } catch (Exception ignored) {
                }
                step(4, failure);
                return trace;
            } catch (Exception outer) {
                return -trace;
            }
        }
    }

    static int probes(int failure) {
        try {
            step(1, failure);
            return 1;
        } catch (Exception ignored) {
        }
        try {
            step(2, failure);
            return 2;
        } catch (Exception ignored) {
        }
        return 3;
    }

    static int protectedExits(int failure) {
        try {
            step(1, failure);
            try {
                step(2, failure);
            } catch (IllegalStateException inner) {
                step(3, failure);
                return -2;
            }
            step(4, failure);
            return 4;
        } catch (RuntimeException outer) {
            return -1;
        } finally {
            trace = trace * 10 + 9;
        }
    }

    public static void main(String[] args) {
        for (int failure = 0; failure <= 6; failure++) {
            trace = 0;
            System.out.println(run(failure));
            trace = 0;
            System.out.println(quiet(failure));
            System.out.println(probes(failure));
            trace = 0;
            System.out.println(protectedExits(failure));
            System.out.println(trace);
        }
    }
}
