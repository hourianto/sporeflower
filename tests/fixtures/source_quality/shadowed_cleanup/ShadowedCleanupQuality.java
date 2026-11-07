public class ShadowedCleanupQuality {
    static int stage, failure, catchFailure, closeFailure;
    static String trace;
    static final RuntimeException runtime = new RuntimeException("body");
    static final Error error = new Error("body");
    static final RuntimeException caughtRuntime = new RuntimeException("catch");
    static final Error caughtError = new Error("catch");
    static final RuntimeException closeRuntime = new RuntimeException("close");
    static final Error closeError = new Error("close");

    static class Resource {
        Resource() { trace += "1"; }
        void close() {
            trace += "4";
            if (closeFailure == 1) throw closeRuntime;
            if (closeFailure == 2) throw closeError;
        }
    }

    static void checkpoint(int current) {
        if (stage != current) return;
        if (failure == 1) throw runtime;
        if (failure == 2) throw error;
    }

    static int playback(boolean early) {
        Resource resource = null;
        try {
            checkpoint(0);
            resource = new Resource();
            synchronized (resource) {
                checkpoint(1);
                trace += "2";
            }
            if (early) return 7;
        } catch (Throwable problem) {
            trace += "3";
            if (catchFailure == 1) throw caughtRuntime;
            if (catchFailure == 2) throw caughtError;
            if (early) return 8;
        } finally {
            if (resource != null) resource.close();
        }
        trace += "5";
        return 9;
    }

    public static void main(String[] arguments) {
        int cases = 0;
        for (stage = 0; stage != 2; stage++) {
            for (failure = 0; failure != 3; failure++) {
                for (catchFailure = 0; catchFailure != 3; catchFailure++) {
                    for (closeFailure = 0; closeFailure != 3; closeFailure++) {
                        for (int early = 0; early != 2; early++) {
                            trace = "";
                            Throwable thrown = null;
                            int result = -1;
                            try { result = playback(early != 0); }
                            catch (Throwable problem) { thrown = problem; }
                            boolean opened = failure == 0 || stage == 1;
                            String expected = opened ? "1" : "";
                            Throwable pending = null;
                            if (failure == 0) expected += "2";
                            else {
                                expected += "3";
                                pending = catchFailure == 1 ? caughtRuntime : catchFailure == 2 ? caughtError : null;
                            }
                            if (opened) {
                                expected += "4";
                                if (closeFailure == 1) pending = closeRuntime;
                                if (closeFailure == 2) pending = closeError;
                            }
                            if (pending == null && early == 0) expected += "5";
                            int expectedResult = pending != null ? -1 : early == 0 ? 9 : failure == 0 ? 7 : 8;
                            if (!trace.equals(expected) || thrown != pending || result != expectedResult) {
                                throw new Error("shadowed cleanup " + stage + ":" + failure + ":" + catchFailure + ":" + closeFailure + ":" + early + " got " + trace + " expected " + expected);
                            }
                            cases++;
                        }
                    }
                }
            }
        }
        System.out.println("shadowed cleanup order, return and exception identity: " + cases);
    }
}
