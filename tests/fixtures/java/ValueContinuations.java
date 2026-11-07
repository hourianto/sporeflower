public final class ValueContinuations {
    private static String trace;
    private static final Object lock = new Object();

    private static void event(String value) {
        trace += value + ";";
    }

    private static void action(int mode) {
        event("action" + mode);
        if (mode == 1) throw new IllegalArgumentException();
        if (mode == 2) throw new IllegalStateException();
        if (mode == 3) throw new Error();
    }

    private static void cleanup(int mode) {
        event("cleanup");
        if (mode == 4) throw new IllegalStateException();
    }

    public static int caughtResult(int mode) {
        boolean restricted = false;
        try {
            action(mode);
        } catch (IllegalArgumentException failure) {
            event("argument");
            restricted = true;
        } catch (IllegalStateException failure) {
            event("state");
        } catch (Error failure) {
            event("error");
        } finally {
            cleanup(mode);
        }
        event("result" + restricted);
        return restricted ? 103 : 100;
    }

    public static int monitoredResult(int mode) {
        boolean restricted = false;
        try {
            synchronized (lock) {
                action(mode);
            }
        } catch (IllegalArgumentException failure) {
            event("argument");
            restricted = true;
        } catch (IllegalStateException failure) {
            event("state");
        } catch (Error failure) {
            event("error");
        } finally {
            cleanup(mode);
        }
        event("result" + restricted);
        return restricted ? 103 : 100;
    }

    public static int cleanupReadsAndWrites(int mode) {
        int value = 7;
        try {
            try {
                action(mode);
                value = 11;
            } catch (IllegalArgumentException failure) {
                value = 13;
            } finally {
                event("before" + value);
                value += 20;
                cleanup(mode);
            }
        } catch (RuntimeException failure) {
            event("caught" + value);
        }
        return value;
    }

    public static int cleanupThrowsBeforeResult(int mode) {
        int value = 7;
        try {
            try {
                action(mode);
            } catch (IllegalArgumentException failure) {
                event("argument");
            } finally {
                cleanup(mode);
            }
            value = 19;
        } catch (RuntimeException failure) {
            event("caught" + value);
        }
        return value;
    }

    public static int search(int mode) {
        boolean found = false;
        for (int index = 0; index < 4; index++) {
            try {
                event("test" + index);
                if (index == mode) {
                    found = true;
                    break;
                }
            } finally {
                event("iteration" + index);
            }
        }
        return found ? 1 : 0;
    }

    public static String probes(int mode) {
        String platform = null;
        first: {
            try {
                action(mode);
                platform = "sample";
                if (mode == 0) return "first";
            } catch (Throwable failure) {
                event("first failed");
                break first;
            }
            try {
                event(platform);
                if (mode == 4) return "second";
            } catch (Throwable failure) {
                event("second failed");
            }
        }
        event("fallback");
        return "fallback";
    }

    public static int cleanupOverridesExit(int mode) {
        int value = 7;
        outer: for (int index = 0; index < 3; index++) {
            event("entry" + value);
            selected: {
                try {
                    if (index == mode) break selected;
                    event("body" + index);
                } finally {
                    event("cleanup" + index);
                    if (index == 0) continue outer;
                    if (index == 2) break outer;
                }
            }
            value = 19;
            event("value" + value);
        }
        return value;
    }

    private static void splitAction(int index, int mode) {
        event("action" + index);
        if (index == mode) throw new IllegalArgumentException();
        if (index == mode + 1) throw new UnsupportedOperationException();
    }

    private static void splitCleanup(int index, int mode) {
        event("cleanup" + index);
        if (index == mode) throw new IllegalStateException();
    }

    public static int exceptionalCleanupCancelsResult(int mode) {
        int value = 7;
        outer: for (int index = 0; index < 3; index++) {
            event("entry" + value);
            try {
                unsupported: {
                    argument: {
                        try {
                            splitAction(index, mode);
                        } catch (IllegalArgumentException failure) {
                            event("argument");
                            break argument;
                        } catch (UnsupportedOperationException failure) {
                            event("unsupported");
                            break unsupported;
                        } finally {
                            splitCleanup(index, mode);
                        }
                        continue outer;
                    }
                    value = 19;
                    continue outer;
                }
                value = 23;
                continue outer;
            } catch (RuntimeException failure) {
                event("caught");
            }
        }
        return value;
    }

    public static int nestedRows(int mode) {
        int[][] rows = new int[][] { new int[] { 1, 2, 3, 4, 5, 6 }, null, new int[] { 7, 8, 9 } };
        int total = 0;
        for (int row = 0; row < rows.length; row++) {
            if (rows[row] != null) {
                for (int column = 0; column < rows[row].length; column += 3) {
                    event("cell" + row + ":" + column);
                    total += rows[row][column] + mode;
                }
            }
        }
        return total;
    }

    public static void main(String[] args) {
        for (int method = 0; method < 9; method++) {
            for (int mode = -1; mode <= 5; mode++) {
                trace = "";
                String result;
                try {
                    switch (method) {
                        case 0: result = "" + caughtResult(mode); break;
                        case 1: result = "" + monitoredResult(mode); break;
                        case 2: result = "" + cleanupReadsAndWrites(mode); break;
                        case 3: result = "" + cleanupThrowsBeforeResult(mode); break;
                        case 4: result = "" + search(mode); break;
                        case 5: result = probes(mode); break;
                        case 6: result = "" + cleanupOverridesExit(mode); break;
                        case 7: result = "" + exceptionalCleanupCancelsResult(mode); break;
                        default: result = "" + nestedRows(mode);
                    }
                } catch (Throwable failure) {
                    result = failure.getClass().getName();
                }
                System.out.println(method + ":" + mode + ":" + trace + ":" + result);
            }
        }
    }
}
