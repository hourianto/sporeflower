public class SharedCatchCoverage {
    static int trace;
    static int mark(int value) { trace = trace * 31 + value; return value; }
    static int effect(int mode, int value) {
        mark(value);
        if (mode == 1) throw new IllegalArgumentException("body");
        if (mode == 2) throw new IllegalStateException("body");
        if (mode == 3) throw new RuntimeException("body");
        return value;
    }

    static int run(int mode, int selector, boolean copy) {
        int first = selector + 10;
        int second = mode + 20;
        try {
            try {
                int value;
                switch (selector & 3) {
                case 0: value = effect(mode, 1); break;
                case 1: value = effect(mode, 2); break;
                case 2: value = effect(mode, 3); break;
                default: value = effect(mode, 4); break;
                }
                return mark(value + first);
            } catch (IllegalArgumentException failure) {
                int savedFirst = first;
                int savedSecond = second;
                mark(70);
                if (copy) mark(savedFirst + savedSecond);
                // This must reach the outer catch, never the sibling below.
                throw new IllegalStateException("handler");
            } catch (IllegalStateException failure) {
                return mark(80);
            }
        } catch (RuntimeException failure) {
            mark(90);
            return failure.getMessage().equals("handler") ? 901 : 902;
        }
    }

    static void quiet() throws InterruptedException { mark(100); }
    static void checkedEffect(boolean fail) throws java.io.IOException {
        mark(110);
        if (fail) throw new java.io.IOException("inner");
    }
    static int impossibleOuter(boolean fail) {
        try {
            try { checkedEffect(fail); }
            catch (java.io.IOException caught) { mark(120); }
            quiet();
        } catch (InterruptedException caught) {
            mark(130);
        }
        return trace;
    }

    public static void main(String[] args) {
        for (int mode = 0; mode < 4; mode++) {
            for (int selector = 0; selector < 8; selector++) {
                for (int copy = 0; copy < 2; copy++) {
                    trace = 0;
                    int value = run(mode, selector, copy != 0);
                    System.out.println(new StringBuffer().append(value).append(':').append(trace).toString());
                }
            }
        }
        for (int fail = 0; fail < 2; fail++) {
            trace = 0;
            System.out.println(impossibleOuter(fail != 0));
        }
    }
}
