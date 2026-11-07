public class ExceptionShapes {
    private static int trace;
    private static int openCount;
    private static Object first = new Object();
    private static Object second = new Object();

    private static int effect(int code, int fail) {
        trace = trace * 31 + code;
        if (code == fail) throw new IllegalArgumentException("step" + code);
        return code;
    }

    public static int nestedMonitors(int mode) {
        synchronized (first) {
            effect(1, mode);
            synchronized (second) {
                if (mode == 2) return effect(2, -1);
                effect(3, mode);
            }
            return effect(4, mode);
        }
    }

    public static int monitoredCatches(int mode) {
        synchronized (first) {
            int value = mode;
            try {
                if (mode < 3) value = effect(5, mode);
                else value = effect(6, mode);
            } catch (IllegalArgumentException failure) {
                value = effect(7, mode);
            }
            try {
                value += effect(8, mode);
            } catch (RuntimeException failure) {
                return effect(9, mode);
            }
            return value;
        }
    }

    public static int cleanupLoop(int mode) {
        try {
            effect(1, mode);
            return 11;
        } finally {
            for (int i = 0; i < 3; i++) {
                switch (i) {
                    case 0: effect(2, mode); break;
                    case 1: effect(3, mode); break;
                    default: effect(4, mode);
                }
            }
        }
    }

    public static int catchLoop(int mode) {
        int sum = 0;
        for (int i = 0; i < 4; i++) {
            try {
                sum += effect(i + 1, mode);
            } catch (IllegalArgumentException failure) {
                if (mode == 2) continue;
                sum -= 10;
            }
        }
        return sum;
    }

    public static int monitorCatchState(int mode) {
        int state = mode + 3;
        try {
            synchronized (first) {
                effect(1, mode);
                state = 5;
                effect(2, mode);
            }
            return state;
        } catch (RuntimeException failure) {
            return state * 7;
        }
    }

    private static void close() {
        trace = trace * 31 + 17;
        if (--openCount < 0) throw new IllegalStateException("already closed");
    }

    public static int closeThenReopen(int mode) {
        openCount = 1;
        try {
            effect(1, mode);
            close();
            openCount++;
            return 23;
        } catch (RuntimeException failure) {
            close();
            throw failure;
        }
    }

    public static int monitorBranches(int mode) {
        outside: {
            synchronized (first) {
                effect(1, -1);
                if (mode == 1) break outside;
                if (mode == 2) return effect(2, -1);
            }
            effect(3, mode);
        }
        return effect(4, mode);
    }

    public static int monitorLoop(int mode) {
        int sum = 0;
        for (int i = 0; i < 4; i++) {
            synchronized (first) {
                sum += effect(i + 1, -1);
                if (i == mode) continue;
                if (mode < 0 && i == 2) break;
            }
            effect(i + 5, mode);
        }
        return sum;
    }

    public static int monitorFinally(int mode) {
        synchronized (first) {
            try { effect(1, mode); }
            finally { effect(2, mode); }
        }
        return effect(3, mode);
    }

    public static int monitorSplitCatch(int mode) {
        synchronized (first) {
            try {
                effect(1, mode);
                if (mode == 2) return 7;
            } catch (IllegalArgumentException failure) {
                effect(3, mode);
            }
        }
        return effect(4, mode);
    }

    public static int monitoredStrings(int mode) {
        synchronized (first) {
            String value = "";
            if (mode > 0) value = "longer";
            effect(1, mode);
            return value.length();
        }
    }

    public static int nestedSwitchReturn(int mode) {
        switch (mode) {
            case 0: return 1;
            case 1: return 2;
            default:
                switch (mode & 3) {
                    case 0: return 3;
                    case 1: return 4;
                    default: return 5;
                }
        }
    }

    public static int nestedMonitorDispatch(int mode) {
        try {
            synchronized (first) {
                try {
                    synchronized (second) { effect(1, mode); }
                } catch (IllegalArgumentException failure) {
                    effect(2, mode);
                }
                effect(3, mode);
            }
            return 17;
        } catch (RuntimeException failure) {
            return effect(4, mode);
        }
    }

    public static int initializedCatchState(int mode) {
        int index, sign;
        if (mode >= 0) { index = mode; sign = 1; }
        else { index = -mode; sign = -1; }
        int value = 7;
        try { value += new int[] { 3, 5, 9 }[index]; }
        catch (ArrayIndexOutOfBoundsException failure) { value += index; }
        return value * sign;
    }

    public static int loopCatch(int mode) {
        int sum = 0;
        for (int i = 0; i < 6; i++) {
            if (i == mode) continue;
            try { sum += effect(i + 1, mode); }
            catch (RuntimeException failure) { }
        }
        return sum;
    }

    public static int retryCatch(int mode) {
        while (true) {
            try { effect(1, mode); return mode; }
            catch (IllegalArgumentException failure) { mode--; }
        }
    }

    public static int switchTryFollow(int mode) {
        try {
            switch (mode) {
                case 1: effect(1, -1); break;
                default: throw new IllegalArgumentException("switch");
            }
        } catch (IllegalArgumentException failure) { return effect(2, -1); }
        return effect(3, -1);
    }

    public static int finallyBranches(int mode) {
        try {
            if (mode < 0) return effect(1, mode);
            if (mode > 3) return effect(2, mode);
            return effect(3, mode);
        } catch (IllegalArgumentException failure) {
            effect(4, mode);
            throw failure;
        } finally { effect(5, mode); }
    }

    public static void main(String[] args) {
        for (int mode = -1; mode < 11; mode++) {
            for (int method = 0; method < 18; method++) {
                trace = 0;
                openCount = 0;
                try {
                    int result;
                    switch (method) {
                        case 0: result = nestedMonitors(mode); break;
                        case 1: result = monitoredCatches(mode); break;
                        case 2: result = cleanupLoop(mode); break;
                        case 3: result = catchLoop(mode); break;
                        case 4: result = monitorCatchState(mode); break;
                        case 5: result = closeThenReopen(mode); break;
                        case 6: result = monitorBranches(mode); break;
                        case 7: result = monitorLoop(mode); break;
                        case 8: result = monitorFinally(mode); break;
                        case 9: result = monitorSplitCatch(mode); break;
                        case 10: result = monitoredStrings(mode); break;
                        case 11: result = nestedSwitchReturn(mode); break;
                        case 12: result = nestedMonitorDispatch(mode); break;
                        case 13: result = initializedCatchState(mode); break;
                        case 14: result = loopCatch(mode); break;
                        case 15: result = retryCatch(mode); break;
                        case 16: result = switchTryFollow(mode); break;
                        default: result = finallyBranches(mode);
                    }
                    System.out.println(result + ":" + trace);
                } catch (Throwable failure) {
                    System.out.println(failure.getClass().getName() + ":" + failure.getMessage() + ":" + trace);
                }
                System.out.println(Thread.holdsLock(first) + ":" + Thread.holdsLock(second));
                System.out.println(openCount);
            }
        }
    }
}
