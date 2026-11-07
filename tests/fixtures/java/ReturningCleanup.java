import java.io.IOException;

public class ReturningCleanup {
    private static final IOException bodyIo = new IOException();
    private static final RuntimeException bodyRuntime = new RuntimeException();
    private static final Error bodyError = new Error();
    private static final IOException cleanupIo = new IOException();
    private static final RuntimeException cleanupRuntime = new RuntimeException();
    private static final Error cleanupError = new Error();
    private static final RuntimeException returnFailure = new RuntimeException();
    private static int mask;
    private static int trace;

    private static void work() throws IOException {
        trace = trace * 10 + 1;
        if ((mask & 1) != 0) throw bodyIo;
        if ((mask & 2) != 0) throw bodyRuntime;
        if ((mask & 4) != 0) throw bodyError;
    }

    private static void close() throws IOException {
        trace = trace * 10 + 2;
        if ((mask & 8) != 0) throw cleanupIo;
        if ((mask & 16) != 0) throw cleanupRuntime;
        if ((mask & 32) != 0) throw cleanupError;
    }

    private static void log() {
        trace = trace * 10 + 3;
        if ((mask & 64) != 0) throw returnFailure;
    }

    private static String result() {
        trace = trace * 10 + 4;
        if ((mask & 64) != 0) throw returnFailure;
        return "changed";
    }

    public static String returning(boolean later) {
        String value = null;
        try {
            work();
            value = "normal";
            if (later) work();
        } catch (IOException caught) {
            log();
            value = "caught";
        } finally {
            try { close(); }
            catch (IOException caught) { log(); }
            return value;
        }
    }

    public static String constant(boolean later) {
        try {
            work();
            if (later) return "ignored";
        } catch (IOException caught) {
            log();
        } finally {
            try { close(); }
            catch (IOException caught) { log(); }
            return "fixed";
        }
    }

    public static String nested(boolean later) {
        try {
            return returning(later);
        } finally {
            trace = trace * 10 + 5;
            if ((mask & 128) != 0) throw cleanupError;
        }
    }

    public static String mutating(boolean later) throws IOException {
        String value = "old";
        try {
            work();
            if (later) value = "normal";
        } finally {
            value = result();
            close();
            return value;
        }
    }

    public static String throwingReturn(boolean later) throws IOException {
        try {
            work();
            if (later) log();
        } finally {
            close();
            return result();
        }
    }

    public static String conditional(boolean later) throws IOException {
        try {
            work();
            return "normal";
        } finally {
            close();
            if (later) return "changed";
        }
    }

    public static String external(boolean enabled) throws IOException {
        if (enabled) {
            try { work(); }
            finally { close(); log(); return "fixed"; }
        }
        return "outside";
    }

    public static String simple(boolean later) throws IOException {
        String value = "before";
        try {
            work();
            if (later) value = "after";
        } finally {
            close();
            return value;
        }
    }

    public static void main(String[] args) {
        for (int bits = 0; bits < 256; bits++) {
            for (int route = 0; route < 16; route++) {
                mask = bits;
                trace = 0;
                String value = null;
                Throwable failure = null;
                boolean later = (route & 1) != 0;
                try {
                    if (route < 2) value = returning(later);
                    else if (route < 4) value = constant(later);
                    else if (route < 6) value = nested(later);
                    else if (route < 8) value = mutating(later);
                    else if (route < 10) value = throwingReturn(later);
                    else if (route < 12) value = conditional(later);
                    else if (route < 14) value = external(later);
                    else value = simple(later);
                } catch (Throwable caught) { failure = caught; }
                int identity = failure == null ? 0 : failure == bodyIo ? 1 : failure == bodyRuntime ? 2 : failure == bodyError ? 3 : failure == cleanupIo ? 4 : failure == cleanupRuntime ? 5 : failure == cleanupError ? 6 : failure == returnFailure ? 7 : -1;
                if (identity < 0) throw new Error("exception identity");
                System.out.println(new StringBuffer().append(value).append(':').append(trace).append(':').append(identity).toString());
            }
        }
    }
}
