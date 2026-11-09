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
    static {
        StaticInitializerDriver.observe(before, StaticDefaultReads.self, StaticDefaultReads.later);
    }
    static int self = StaticInitializerDriver.mark(StaticDefaultReads.self + 1);
    static int later = StaticInitializerDriver.mark(4);
    static int callback = readPending();
    static int pending = StaticInitializerDriver.mark(5);

    static int readPending() { return pending; }
}

class StaticLoopBoundary {
    static int[] first = new int[] { StaticInitializerDriver.mark(20) };
    static int afterLoop;
    static int[] last;
    static {
        for (int index = 0; index < 2; index++) {
            StaticInitializerDriver.observe(first[0], index, afterLoop);
        }
        afterLoop = StaticInitializerDriver.mark(21);
        StaticInitializerDriver.observe(first[0], afterLoop, 0);
        last = new int[] { StaticInitializerDriver.mark(22), StaticInitializerDriver.mark(23) };
    }
}

class StaticLocalBoundaries {
    static int afterDead;
    static int sharedPublication;
    static int tail;
    static {
        int expired = StaticInitializerDriver.mark(8);
        StaticInitializerDriver.observe(expired, expired, afterDead);
        afterDead = StaticInitializerDriver.mark(9);
        int shared = StaticInitializerDriver.mark(10);
        StaticInitializerDriver.observe(shared, shared, afterDead);
        sharedPublication = StaticInitializerDriver.mark(11);
        StaticInitializerDriver.observe(shared, sharedPublication, tail);
        tail = StaticInitializerDriver.mark(12);
    }
}

class StaticRepeatedWrites {
    static int repeated;
    static int later;
    static {
        repeated = StaticInitializerDriver.mark(30);
        StaticInitializerDriver.observe(repeated, later, 0);
        repeated = StaticInitializerDriver.mark(31);
        later = StaticInitializerDriver.mark(32);
    }
}

class StaticEarlyExit {
    static int prefix = StaticInitializerDriver.mark(39);
    static int tail;
    static {
        if (StaticInitializerDriver.mark(40) > 0) {
            // The fixture rewrites this call into a class-initializer return.
            StaticInitializerDriver.earlyExit();
        }
        tail = StaticInitializerDriver.mark(41);
    }
}

class StaticFailedInitialization {
    static int published = StaticInitializerDriver.mark(6);
    static {
        StaticInitializerDriver.observe(published, StaticFailedInitialization.failure,
            StaticFailedInitialization.unreached);
    }
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

    static void earlyExit() {}

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
        System.out.println("loop:" + StaticLoopBoundary.first[0] + ":"
            + StaticLoopBoundary.afterLoop + ":" + StaticLoopBoundary.last[0] + ":"
            + StaticLoopBoundary.last[1]);
        System.out.println("locals:" + StaticLocalBoundaries.afterDead + ":"
            + StaticLocalBoundaries.sharedPublication + ":" + StaticLocalBoundaries.tail);
        System.out.println("repeated:" + StaticRepeatedWrites.repeated + ":"
            + StaticRepeatedWrites.later);
        System.out.println("early:" + StaticEarlyExit.prefix + ":" + StaticEarlyExit.tail);
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
