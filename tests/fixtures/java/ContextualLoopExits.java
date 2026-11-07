public class ContextualLoopExits {
    private static int trace;
    private static int calls;
    private static int fail;
    private int plainSelector;
    private volatile int volatileSelector;

    private static int mark(int value) {
        trace = trace * 31 + value;
        if (++calls == fail) throw new IllegalStateException("site " + value);
        return value;
    }

    public static int scan(int mode, int limit) {
        int sum = 0;
        int i = 0;
        while (i < limit) {
            int value = mark(i + mode);
            if (value < 0 || value > 7) {
                mark(40);
                return -1;
            }
            if (value == 2 || value == 6) {
                mark(41);
                return -2;
            }
            if (value == 5) {
                if (mode != 0) mark(42);
                break;
            }
            sum += mark(10 + value);
            i++;
        }
        mark(80 + i);
        return sum;
    }

    public static int restart(int mode, int limit) {
        int pass = 0;
        int sum = 0;
        outer: while (pass < limit) {
            pass++;
            for (int i = 0; i < 4; i++) {
                int value = mark(i + pass + mode);
                if (value == 2 || value == 7) {
                    sum += mark(50);
                    continue outer;
                }
                if (value == 4 || value == 9) {
                    sum -= mark(51);
                    continue outer;
                }
                if (value > 10) return mark(52);
                sum += value;
            }
            mark(60);
            break;
        }
        return mark(90) + sum + pass;
    }

    public static int nestedBreaks(int mode, int limit) {
        int sum = 0;
        for (int i = 0; i < limit; i++) {
            if (mode == i) continue;
            for (int j = 0; j < 4; j++) {
                if (j == mode) break;
                sum += mark(10 + i + j);
                for (int k = 0; k < 3; k++) {
                    if (k + i == mode) {
                        sum += mark(30);
                        break;
                    }
                    sum += mark(40 + k);
                }
            }
        }
        return sum;
    }

    public static int cleanupFallback(int mode) {
        try {
            try {
                mark(60);
                if ((mode & 1) != 0) {
                    int result = mark(61);
                    return result;
                }
            } catch (IllegalStateException failure) {
                mark(62);
            }
        } finally {
            mark(63);
        }
        return mark(64);
    }

    public static int selectedDecision(int mode, int limit) {
        decision: {
            if ((mark(mode) & 1) == 0) {
                if (mark(limit + 10) > 12) break decision;
            } else if (mark(limit + 20) < 23) {
                break decision;
            }
            mark(50);
        }
        return mark(60);
    }

    public static int guardedResult(int mode, int limit) {
        int result = -1;
        search: {
            if ((mark(mode) & 1) == 0) {
                if (mark(limit + 10) > 12) break search;
            } else if (mark(limit + 20) < 23) {
                break search;
            }
            for (int index = 0; index < limit; index++) {
                if (mark(index + 30) != mode + 30) continue;
                result = index;
                mark(70);
                break;
            }
        }
        return mark(result + 80);
    }

    public int redundantDecision(int limit) {
        decision: {
            if (this.plainSelector == 0) {
                if (limit > 2) break decision;
            } else if (limit > 2) break decision;
            mark(50);
        }
        return mark(60);
    }

    public int volatileDecision(int limit) {
        decision: {
            if (this.volatileSelector == 0) {
                if (limit > 2) break decision;
            } else if (limit > 2) break decision;
            mark(50);
        }
        return mark(60);
    }

    public static int effectfulDecision(int mode, int limit) {
        decision: {
            if (mark(mode) == 0) {
                if (limit > 2) break decision;
            } else if (limit > 2) break decision;
            mark(50);
        }
        return mark(60);
    }

    public static void main(String[] args) {
        ContextualLoopExits receiver = new ContextualLoopExits();
        for (int mode = -1; mode < 9; mode++) {
            receiver.plainSelector = mode;
            receiver.volatileSelector = mode;
            for (int limit = 0; limit < 6; limit++) {
                for (int site = 0; site < 14; site++) {
                    for (int method = 0; method < 9; method++) {
                        trace = 0;
                        calls = 0;
                        fail = site;
                        try {
                            int result;
                            if (method == 0) result = scan(mode, limit);
                            else if (method == 1) result = restart(mode, limit);
                            else if (method == 2) result = nestedBreaks(mode, limit);
                            else if (method == 3) result = cleanupFallback(mode);
                            else if (method == 4) result = selectedDecision(mode, limit);
                            else if (method == 5) result = guardedResult(mode, limit);
                            else if (method == 6) result = receiver.redundantDecision(limit);
                            else if (method == 7) result = receiver.volatileDecision(limit);
                            else result = effectfulDecision(mode, limit);
                            System.out.println(result + ":" + trace + ":" + calls);
                        } catch (Throwable failure) {
                            System.out.println(failure.getClass().getName() + ":" + failure.getMessage() + ":" + trace + ":" + calls);
                        }
                    }
                }
            }
        }
    }
}
