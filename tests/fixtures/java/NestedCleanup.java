public class NestedCleanup {
    private static final IllegalArgumentException bodyFailure = new IllegalArgumentException();
    private static final RuntimeException catchFailure = new RuntimeException();
    private static final RuntimeException innerFailure = new RuntimeException();
    private static final RuntimeException outerFailure = new RuntimeException();
    private static int failures;
    private static int trace;

    private static void step(int stage) {
        trace = trace * 10 + stage;
        if ((failures & (1 << stage)) == 0) return;
        if (stage <= 2) throw bodyFailure;
        if (stage == 3) throw catchFailure;
        if (stage == 4) throw innerFailure;
        throw outerFailure;
    }

    public static int nested(boolean early) {
        try {
            try {
                if (early) {
                    step(1);
                    return 7;
                }
                step(2);
                return 9;
            } catch (IllegalArgumentException caught) {
                if (caught != bodyFailure) throw new Error("identity");
                step(3);
                return 11;
            } finally {
                step(4);
            }
        } finally {
            step(5);
        }
    }

    public static int shared(boolean enabled) {
        int count = 0;
        while (count < 2) {
            if (enabled) {
                try {
                    step(1);
                } catch (IllegalArgumentException caught) {
                    step(3);
                } finally {
                    try { step(4); }
                    catch (RuntimeException ignored) { }
                }
            }
            step(5);
            count++;
        }
        return count;
    }

    public static int conditionalCleanup(boolean skip) throws Throwable {
        try {
            synchronized (bodyFailure) {
                if (skip) return 13;
            }
            step(1);
        } catch (Throwable caught) {
            step(4);
            throw caught;
        }
        step(4);
        return 17;
    }

    public static int handledInside(boolean early) {
        try {
            try {
                step(1);
                if (early) {
                    try { step(2); }
                    catch (Throwable caught) { step(3); }
                    return 19;
                }
            } catch (IllegalArgumentException caught) {
                try { step(2); }
                catch (Throwable ignored) { }
                step(3);
            }
            return 23;
        } finally {
            step(4);
        }
    }

    public static void main(String[] args) {
        for (int mask = 0; mask < 64; mask++) {
            for (int branch = 0; branch < 2; branch++) {
                failures = mask;
                trace = 0;
                int result = 0;
                Throwable failure = null;
                try { result = nested(branch != 0); }
                catch (Throwable caught) { failure = caught; }
                int identity = failure == null ? 0 : failure == bodyFailure ? 1 : failure == catchFailure ? 2 : failure == innerFailure ? 3 : failure == outerFailure ? 4 : -1;
                if (identity < 0) throw new Error("unexpected failure");
                System.out.println(result + ":" + identity + ":" + trace);
                trace = 0;
                result = 0;
                failure = null;
                try { result = handledInside(branch != 0); }
                catch (Throwable caught) { failure = caught; }
                identity = failure == null ? 0 : failure == bodyFailure ? 1 : failure == catchFailure ? 2 : failure == innerFailure ? 3 : failure == outerFailure ? 4 : -1;
                if (identity < 0) throw new Error("nested handled failure");
                System.out.println(result + ":" + identity + ":" + trace);
                trace = 0;
                result = 0;
                failure = null;
                try { result = shared(branch != 0); }
                catch (Throwable caught) { failure = caught; }
                identity = failure == null ? 0 : failure == bodyFailure ? 1 : failure == catchFailure ? 2 : failure == innerFailure ? 3 : failure == outerFailure ? 4 : -1;
                if (identity < 0) throw new Error("unexpected shared failure");
                System.out.println(result + ":" + identity + ":" + trace);
                trace = 0;
                result = 0;
                failure = null;
                try { result = conditionalCleanup(branch != 0); }
                catch (Throwable caught) { failure = caught; }
                identity = failure == null ? 0 : failure == bodyFailure ? 1 : failure == catchFailure ? 2 : failure == innerFailure ? 3 : failure == outerFailure ? 4 : -1;
                if (identity < 0 || Thread.holdsLock(bodyFailure)) throw new Error("conditional cleanup");
                System.out.println(result + ":" + identity + ":" + trace);
            }
        }
    }
}
