public class LoopExitBoundaries {
    private static final Object lock = new Object();
    private static int trace;
    private static int finishes;
    private static int cursor;

    private static boolean keep(int limit, int value) {
        trace = trace * 31 + value + 1;
        if (limit == 6 && value == 2) throw new IllegalArgumentException("guard");
        return value < limit;
    }

    private static int finish(int value) {
        finishes++;
        trace = trace * 31 + 41;
        if (value == 4) throw new IllegalStateException("result");
        return value * 17 + trace;
    }

    private static int returning(int limit) {
        for (int i = 0; ; i++) {
            if (!keep(limit, i)) return finish(i);
            trace = trace * 31 + 11;
        }
    }

    private static int anotherBreak(int limit) {
        int i = 0;
        for (; ; i++) {
            if (!keep(limit, i)) return finish(i);
            if (i == 2) break;
            trace = trace * 31 + 13;
        }
        return finish(i + 20);
    }

    private static int outerTransfers(int limit) {
        int sum = 0;
        outer: for (int i = 0; i < 3; i++) {
            for (int j = 0; ; j++) {
                if (!keep(limit, j)) continue outer;
                if (j == 2 && i == 1) break outer;
                sum += i * 10 + j;
            }
        }
        return finish(sum);
    }

    private static int cleanup(int limit) {
        synchronized (lock) {
            try {
                for (int i = 0; ; i++) {
                    if (!keep(limit, i)) return finish(i);
                    trace = trace * 31 + 17;
                }
            } finally {
                trace = trace * 31 + 19;
                if (limit == 5) throw new IllegalStateException("cleanup");
            }
        }
    }

    private static boolean take(int limit) {
        trace = trace * 31 + cursor;
        return ++cursor < limit;
    }

    private static int empty(int limit) {
        cursor = 0;
        while (take(limit)) {
        }
        return finish(cursor);
    }

    private static boolean placement(int limit) {
        for (int i = 0; i < limit; i++) {
            if (!keep(limit + 1, i)) return false;
            for (int j = 0; j < 3; j++) {
                if (i + j == 4) return false;
            }
            if (i == 5) return false;
        }
        for (int i = 0; i < 3; i++) {
            for (int j = 0; j < 3; j++) {
                if (limit == i + j) return false;
            }
        }
        return true;
    }

    private static int literalCleanup(int limit) {
        synchronized (lock) {
            done: {
                try {
                    for (int i = 0; i < 5; i++) {
                        if (!keep(limit, i)) break done;
                        trace = trace * 31 + i;
                    }
                } finally {
                    trace = trace * 31 + 23;
                    if (limit == 3) throw new IllegalStateException("literal cleanup");
                }
            }
            return 17;
        }
    }

    private static int localAfterCleanup(int limit) {
        int result = 1;
        done: {
            try {
                for (int i = 0; i < 5; i++) {
                    if (!keep(limit, i)) break done;
                    result = 5;
                }
            } finally {
                result = limit + 30;
                trace = trace * 31 + result;
            }
        }
        return result;
    }

    public static void main(String[] args) {
        for (int limit = 0; limit < 8; limit++) {
            for (int method = 0; method < 8; method++) {
                trace = 0;
                finishes = 0;
                try {
                    int result;
                    if (method == 0) result = returning(limit);
                    else if (method == 1) result = anotherBreak(limit);
                    else if (method == 2) result = outerTransfers(limit);
                    else if (method == 3) result = cleanup(limit);
                    else if (method == 4) result = empty(limit);
                    else if (method == 5) result = placement(limit) ? 1 : 0;
                    else if (method == 6) result = literalCleanup(limit);
                    else result = localAfterCleanup(limit);
                    System.out.println(result + ":" + trace + ":" + finishes);
                } catch (Throwable failure) {
                    System.out.println(failure.getClass().getName() + ":" + failure.getMessage() + ":" + trace + ":" + finishes);
                }
            }
        }
    }
}
