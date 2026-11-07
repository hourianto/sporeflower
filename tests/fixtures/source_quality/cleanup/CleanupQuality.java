import java.io.IOException;

public class CleanupQuality {
    static int stage, failure, closeFirst, closeSecond, catchFailure;
    static String trace;
    static final IOException io = new IOException("body");
    static final RuntimeException runtime = new RuntimeException("body");
    static final Error error = new Error("body");
    static final RuntimeException caughtRuntime = new RuntimeException("catch");
    static final Error caughtError = new Error("catch");
    static final IOException closeIo = new IOException("close");
    static final RuntimeException closeRuntime = new RuntimeException("close");
    static final Error closeError = new Error("close");

    static class Stream {
        final int id;
        Stream(int id) { this.id = id; trace += id; }
        void close() throws IOException {
            trace += id + 3;
            int problem = id == 1 ? closeFirst : closeSecond;
            if (problem == 1) throw closeIo;
            if (problem == 2) throw closeRuntime;
            if (problem == 3) throw closeError;
        }
    }

    static void checkpoint(int current) throws IOException {
        if (stage != current) return;
        if (failure == 1) throw io;
        if (failure == 2) throw runtime;
        if (failure == 3) throw error;
    }

    static void load() {
        boolean complete = false;
        Stream first = null;
        Stream second = null;
        try {
            checkpoint(0);
            first = new Stream(1);
            checkpoint(1);
            second = new Stream(2);
            checkpoint(2);
            for (int index = 0; index < 2; index++) {
                try {
                    if (index == 0) checkpoint(3);
                } catch (IOException inner) {
                    trace += "6";
                }
            }
            complete = true;
        } catch (IOException problem) {
            trace += "3";
            if (catchFailure == 1) throw caughtRuntime;
            if (catchFailure == 2) throw caughtError;
        } finally {
            try {
                if (first != null) first.close();
                if (second != null) second.close();
            } catch (IOException ignored) {}
        }
        trace += complete ? "9" : "8";
    }

    public static void main(String[] arguments) {
        int cases = 0;
        for (stage = 0; stage != 4; stage++) {
            for (failure = 0; failure != 4; failure++) {
                for (closeFirst = 0; closeFirst != 4; closeFirst++) {
                    for (closeSecond = 0; closeSecond != 4; closeSecond++) {
                        for (catchFailure = 0; catchFailure != 3; catchFailure++) {
                            trace = "";
                            Throwable thrown = null;
                            try { load(); } catch (Throwable problem) { thrown = problem; }
                            int opened = failure == 0 || stage > 1 ? 2 : stage;
                            String expected = opened == 0 ? "" : opened == 1 ? "1" : "12";
                            Throwable pending = failure == 2 ? runtime : failure == 3 ? error : null;
                            boolean inner = stage == 3 && failure == 1;
                            if (inner) expected += "6";
                            else if (failure == 1) {
                                expected += "3";
                                pending = catchFailure == 1 ? caughtRuntime : catchFailure == 2 ? caughtError : null;
                            }
                            int closing = 0;
                            if (opened > 0) { expected += "4"; closing = closeFirst; }
                            if (opened > 1 && closing == 0) { expected += "5"; closing = closeSecond; }
                            if (closing == 2) pending = closeRuntime;
                            if (closing == 3) pending = closeError;
                            if (pending == null) expected += failure == 0 || inner ? "9" : "8";
                            if (!trace.equals(expected) || thrown != pending) {
                                throw new Error("cleanup " + stage + ":" + failure + ":" + closeFirst + ":" + closeSecond + ":" + catchFailure + " got " + trace + " expected " + expected);
                            }
                            cases++;
                        }
                    }
                }
            }
        }
        System.out.println("cleanup order and exception identity: " + cases);
    }
}
