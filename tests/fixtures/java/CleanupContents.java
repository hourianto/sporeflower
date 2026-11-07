public class CleanupContents {
    private static final RuntimeException bodyFailure = new RuntimeException();
    private static final RuntimeException closeFailure = new RuntimeException();
    private static final Error closeError = new Error();
    private static int mask;
    private static int trace;

    private static Object open() {
        trace = trace * 10 + 1;
        if ((mask & 1) != 0) throw bodyFailure;
        return (mask & 2) == 0 ? new Object() : null;
    }
    private static void work() {
        trace = trace * 10 + 2;
        if ((mask & 4) != 0) throw bodyFailure;
    }
    private static void close(Object value) {
        trace = trace * 10 + (value == null ? 3 : 4);
        if ((mask & 8) != 0) throw closeFailure;
        if ((mask & 16) != 0) throw closeError;
    }
    private static void clear() {
        trace = trace * 10 + 5;
        if ((mask & 8) != 0) throw closeFailure;
    }
    private static void finish() {
        trace = trace * 10 + 6;
        if ((mask & 16) != 0) throw closeError;
    }
    public static int postCleanup(boolean entered) {
        if (entered) { try { work(); } finally { clear(); finish(); } }
        return 23;
    }
    public static int insideCleanup(boolean entered) {
        if (entered) { try { work(); } finally { clear(); finish(); } }
        return 29;
    }

    public static int result() {
        Object resource = null;
        boolean success = true;
        try { resource = open(); work(); }
        catch (RuntimeException caught) { success = false; }
        finally {
            try { work(); } catch (Throwable ignored) {}
            if (resource != null) {
                try { close(resource); } catch (Throwable ignored) {}
            }
        }
        return success ? 11 : 17;
    }

    public static int flag() {
        boolean success = true;
        try { work(); }
        catch (RuntimeException caught) { success = false; }
        finally { try { work(); } catch (Throwable ignored) {} }
        return success ? 11 : 17;
    }

    public static int optionalCleanup() {
        try { work(); }
        catch (RuntimeException caught) { finish(); }
        finally { if ((mask & 2) != 0) clear(); }
        return 31;
    }

    public static boolean unopened(boolean early) {
        Object resource = null;
        try {
            if (early) return false;
            resource = open();
            work();
            return true;
        } finally {
            try { work(); } catch (Throwable ignored) {}
            close(resource);
        }
    }

    public static int mutable() {
        Object resource = null;
        int result = 5;
        try { resource = open(); return result; }
        finally {
            result = 7;
            if ((mask & 32) != 0) resource = null;
            try { work(); } catch (Throwable ignored) {}
            close(resource);
        }
    }

    public static int optimizedNull(boolean early) {
        Object resource = null;
        try {
            work();
            if (early) return 37;
            resource = open();
            work();
            return 41;
        } finally {
            if (resource != null) close(resource);
            finish();
        }
    }

    public static int convergingCleanup(boolean early) {
        Object first = open();
        Object skipped = null;
        Object last = open();
        try {
            work();
            if (early) {
                skipped = open();
                work();
                return 89;
            }
            return 97;
        } finally {
            if (first != null) close(first);
            if (skipped != null) close(skipped);
            if (last != null) close(last);
            finish();
        }
    }

    public static int overwrittenNull(boolean early) {
        Object resource = null;
        try {
            work();
            if (early) return 43;
            return 47;
        } finally {
            resource = open();
            if (resource != null) close(resource);
            finish();
        }
    }

    public static int emptyEarly(boolean early) {
        Object resource = null;
        try {
            work();
            if (early) return 79;
            resource = open();
            work();
            return 83;
        } finally {
            if (resource != null) close(resource);
        }
    }

    public static int optimizedLoop(boolean early) {
        Object resource = null;
        int redirects = 2;
        try {
            while (true) {
                resource = null;
                work();
                if (redirects-- > 0) { finish(); continue; }
                if (early) return 71;
                resource = open();
                work();
                return 73;
            }
        } finally {
            if (resource != null) close(resource);
            finish();
        }
    }

    public static int unknownNull(boolean early) {
        Object resource = open();
        try {
            work();
            if (early) return 53;
            return 59;
        } finally {
            if (resource != null) close(resource);
            finish();
        }
    }

    public static int throwingGuard(boolean early) {
        Object[] resource = early ? null : new Object[] {new Object()};
        try {
            work();
            if (early) return 61;
            return 67;
        } finally {
            if (resource[0] != null) clear();
            finish();
        }
    }

    public static void main(String[] ignored) {
        for (int value = 0; value < 64; value++) {
            for (int test = 0; test < 22; test++) {
                mask = value;
                trace = 0;
                int result = 0;
                Throwable failure = null;
                try {
                    if (test == 0) result = result();
                    else if (test == 1) result = unopened(false) ? 1 : 0;
                    else if (test == 2) result = unopened(true) ? 1 : 0;
                    else if (test == 3) result = mutable();
                    else if (test == 4) result = flag();
                    else if (test == 5) result = postCleanup((mask & 32) != 0);
                    else if (test == 6) result = insideCleanup((mask & 32) != 0);
                    else if (test == 7) result = optionalCleanup();
                    else if (test < 10) result = optimizedNull(test == 8);
                    else if (test < 12) result = overwrittenNull(test == 10);
                    else if (test < 14) result = unknownNull(test == 12);
                    else if (test < 16) result = throwingGuard(test == 14);
                    else if (test < 18) result = optimizedLoop(test == 16);
                    else if (test < 20) result = emptyEarly(test == 18);
                    else result = convergingCleanup(test == 20);
                } catch (Throwable caught) { failure = caught; }
                int identity = failure == null ? 0 : failure == bodyFailure ? 1 : failure == closeFailure ? 2 : failure == closeError ? 3 : failure instanceof NullPointerException ? 4 : -1;
                if (identity < 0) throw new Error("identity");
                System.out.println(new StringBuffer().append(result).append(':').append(trace).append(':').append(identity).toString());
            }
        }
    }
}
