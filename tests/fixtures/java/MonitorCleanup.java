public class MonitorCleanup {
    private static final Object bodyLock = new Object();
    private static final Object outerLock = new Object();
    private static final Object innerLock = new Object();
    private static final IllegalArgumentException typed = new IllegalArgumentException();
    private static final IllegalStateException handled = new IllegalStateException();
    private static final Error escaping = new Error();
    private static Object notification;
    private static int failAt;
    private static String trace;

    private static void step(int stage) {
        trace += stage + "/" + MonitorCleanupProbe.held(bodyLock) + "/"
            + MonitorCleanupProbe.held(outerLock) + "/" + MonitorCleanupProbe.held(innerLock) + ";";
        if (stage != failAt) return;
        if (stage == 1) throw typed;
        if (stage == 6) throw handled;
        throw escaping;
    }

    public static int nested(int mode) {
        try {
            synchronized (bodyLock) {
                step(1);
                if (mode == 1) throw typed;
                if (mode == 2) throw escaping;
            }
            if (mode == 3) return 7;
            return 11;
        } catch (IllegalArgumentException caught) {
            if (caught != typed) throw new Error("typed identity");
            synchronized (bodyLock) { step(2); }
            return 13;
        } finally {
            step(3);
            synchronized (outerLock) {
                step(4);
                synchronized (innerLock) {
                    step(5);
                    try { step(6); }
                    catch (IllegalStateException caught) {
                        if (caught != handled) throw new Error("cleanup identity");
                    }
                }
                step(7);
            }
            step(8);
        }
    }

    public static void conditional(int mode) {
        if (mode == 3) return;
        try {
            step(1);
            if (mode == 1) throw typed;
            if (mode == 2) throw escaping;
        } catch (IllegalArgumentException caught) {
            step(2);
        } finally {
            step(3);
            if (notification != null) {
                synchronized (notification) {
                    notification.notifyAll();
                    step(4);
                }
            }
            step(8);
        }
    }

    // Identical bodies do not make these copies of one cleanup: the normal
    // and exceptional paths deliberately acquire different locks.
    public static void differentLocks(int mode) throws Throwable {
        try {
            step(1);
            if (mode != 0) throw escaping;
        } catch (Throwable caught) {
            synchronized (outerLock) { step(4); }
            throw caught;
        }
        synchronized (innerLock) { step(4); }
    }

    // The normal cleanup and the following monitor acquisition share one
    // bytecode block. Recovering only the cleanup prefix must retain the
    // complete synchronized continuation, including its nested monitor.
    public static void cleanupBeforeMonitor(int mode) throws Throwable {
        try {
            step(1);
            if (mode != 0) throw escaping;
        } catch (Throwable caught) {
            step(3);
            throw caught;
        }
        step(3);
        synchronized (outerLock) {
            step(4);
            synchronized (innerLock) { step(5); }
            step(7);
        }
        step(8);
    }

    public static void main(String[] args) {
        for (int method = 0; method < 4; method++) {
            for (int mode = 0; mode < 4; mode++) {
                for (failAt = 0; failAt <= 8; failAt++) {
                    for (int present = 0; present < 2; present++) {
                        notification = present == 0 ? null : outerLock;
                        trace = "";
                        int result = 0;
                        Throwable failure = null;
                        try {
                            if (method == 0) result = nested(mode);
                            else if (method == 1) conditional(mode);
                            else if (method == 2) differentLocks(mode);
                            else cleanupBeforeMonitor(mode);
                        } catch (Throwable caught) { failure = caught; }
                        int identity = failure == null ? 0 : failure == typed ? 1
                            : failure == handled ? 2 : failure == escaping ? 3 : -1;
                        if (identity < 0 || MonitorCleanupProbe.held(bodyLock)
                            || MonitorCleanupProbe.held(outerLock) || MonitorCleanupProbe.held(innerLock))
                            throw new Error("identity or ownership");
                        System.out.println(method + ":" + mode + ":" + failAt + ":"
                            + present + ":" + result + ":" + identity + ":" + trace);
                    }
                }
            }
        }
    }
}

class MonitorCleanupProbe {
    public static boolean held(Object lock) { return Thread.holdsLock(lock); }
}
