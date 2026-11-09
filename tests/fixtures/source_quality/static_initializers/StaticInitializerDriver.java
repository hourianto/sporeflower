class StaticArrayDeclarations {
    static final byte[] BYTES = new byte[] { -128, 0, 127 };
    static final int[] SINGLE = new int[] { 7 };
    static final int[][] ROWS = new int[][] { { 1 }, { 2, 3 } };
    static final String[] WORDS = new String[] { "first", "second" };
    static final StringBuffer TEXT = new StringBuffer("ready");
}

class StaticPublicationOrder {
    // The field table order differs from the order of the original writes.
    static int second;
    static int first;
    static int last;
    static {
        first = StaticInitializerDriver.mark(1);
        second = StaticInitializerDriver.mark(first + 1);
        StaticInitializerDriver.observe(first, second, last);
        last = StaticInitializerDriver.mark(3);
    }
}

class StaticDefaultReads {
    static int before = StaticDefaultReads.later;
    static int self = StaticInitializerDriver.mark(StaticDefaultReads.self + 1);
    static int later = StaticInitializerDriver.mark(4);
    static int callback = readPending();
    static int pending = StaticInitializerDriver.mark(5);

    static int readPending() { return pending; }
}

class StaticFailedInitialization {
    static int published = StaticInitializerDriver.mark(6);
    static int failure = StaticInitializerDriver.fail();
    static int unreached = StaticInitializerDriver.mark(7);
}

public class StaticInitializerDriver {
    static String trace = "";

    static int mark(int value) {
        trace += "mark:" + value + ";";
        return value;
    }

    static void observe(int first, int second, int last) {
        trace += "observe:" + first + ":" + second + ":" + last + ";";
    }

    static int fail() {
        trace += "fail;";
        throw new IllegalStateException("stop");
    }

    public static void main(String[] args) {
        System.out.println("arrays:" + StaticArrayDeclarations.BYTES[0] + ":"
            + StaticArrayDeclarations.BYTES[2] + ":" + StaticArrayDeclarations.SINGLE[0]
            + ":" + StaticArrayDeclarations.ROWS[0][0] + ":" + StaticArrayDeclarations.ROWS[1][1]
            + ":" + StaticArrayDeclarations.WORDS[1] + ":" + StaticArrayDeclarations.TEXT);
        System.out.println("ordered:" + StaticPublicationOrder.first + ":"
            + StaticPublicationOrder.second + ":" + StaticPublicationOrder.last);
        System.out.println("defaults:" + StaticDefaultReads.before + ":" + StaticDefaultReads.self
            + ":" + StaticDefaultReads.later + ":" + StaticDefaultReads.callback
            + ":" + StaticDefaultReads.pending);
        for (int attempt = 0; attempt < 2; attempt++) {
            try {
                System.out.println(StaticFailedInitialization.unreached);
            } catch (Throwable failure) {
                System.out.println(failure.getClass().getName());
            }
        }
        System.out.println(trace);
    }
}
