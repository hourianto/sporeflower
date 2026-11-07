public class OwnedLoopContinuation {
    private static int trace;
    private static int steps;

    private static void touch(int site, int mode) {
        trace = trace * 31 + site;
        if (++steps > 100) throw new AssertionError("iteration budget");
        if (mode == 5 && site == 52) throw new IllegalStateException("tail");
    }

    public static int sharedTail(int mode, int limit) {
        int sum = 0;
        int i;
        for (i = 0; i < limit; i++) {
            touch(10 + i, mode);
            int delta;
            switch ((i + mode) & 3) {
            case 0: continue;
            case 1: delta = 3; break;
            case 2: delta = 5; break;
            default: delta = 7;
            }
            touch(50 + i, mode);
            sum += delta;
        }
        return i * 1000 + sum;
    }

    public static int directHeader(int mode, int limit) {
        int i = 0;
        while (i < limit) {
            touch(20 + i, mode);
            if ((steps & 1) == 0) {
                i++;
                continue;
            }
            i += 2;
        }
        return i;
    }

    public static int nested(int mode, int limit) {
        int sum = 0;
        outer: for (int i = 0; i < limit; i++) {
            for (int j = 0; j < 3; j++) {
                touch(30 + i * 3 + j, mode);
                if (((i + j + mode) % 5) == 0) continue outer;
                sum += i + j;
            }
            sum += 100;
        }
        return sum;
    }

    public static int observedFinally(int mode, int limit) {
        int sum = 0;
        int i = 0;
        try {
            for (; i < limit; i++) {
                touch(10 + i, mode);
                if (((i + mode) & 3) == 0) continue;
                if (mode == 6 && i == 3) return sum;
                touch(50 + i, mode);
                sum += i;
            }
            return sum;
        } finally {
            touch(900 + i, mode);
        }
    }

    public static int exitCorridor(int mode, int limit) {
        int max = 0;
        for (int i = 0; i < limit; i++) {
            touch(10 + i, mode);
            int value = i * 3 + mode;
            if (value <= max) continue;
            if (value <= 8) {
                max = value;
                continue;
            }
            touch(52, mode);
            max = 8;
            break;
        }
        touch(80 + max, mode);
        return max;
    }

    public static int conditionalSwitchFollow(int mode, int limit) {
        int sum = 0;
        for (int i = 0; i < limit; i++) {
            boolean ready;
            switch ((i + mode) & 3) {
            case 0: continue;
            case 1: touch(10 + i, mode); ready = i > 1; break;
            default:
                switch (mode) {
                case 3: touch(30 + i, mode); ready = true; break;
                default: ready = (i & 1) == 0;
                }
            }
            if (!ready) continue;
            touch(50 + i, mode);
            sum += i;
        }
        return sum;
    }

    public static int optionalSwitchArm(int mode, int limit) {
        int sum = 0;
        for (int i = 0; i < limit; i++) {
            if ((i & 1) == 0) {
                boolean ready;
                switch ((i + mode) & 3) {
                case 0: continue;
                case 1: touch(10 + i, mode); ready = i > 1; break;
                default: touch(30 + i, mode); ready = mode != 4;
                }
                if (!ready) continue;
                touch(50 + i, mode);
            }
            sum += i;
            if (mode == 6 && i == 4) return sum;
        }
        return sum;
    }

    public static void main(String[] args) {
        for (int mode = 0; mode < 7; mode++) {
            for (int limit = 0; limit < 7; limit++) {
                for (int method = 0; method < 7; method++) {
                    trace = 0;
                    steps = 0;
                    try {
                        int value;
                        if (method == 0) value = sharedTail(mode, limit);
                        else if (method == 1) value = directHeader(mode, limit);
                        else if (method == 2) value = nested(mode, limit);
                        else if (method == 3) value = observedFinally(mode, limit);
                        else if (method == 4) value = exitCorridor(mode, limit);
                        else if (method == 5) value = conditionalSwitchFollow(mode, limit);
                        else value = optionalSwitchArm(mode, limit);
                        System.out.println(value + ":" + trace + ":" + steps);
                    } catch (Throwable failure) {
                        System.out.println(failure.getClass().getName() + ":" + failure.getMessage() + ":" + trace + ":" + steps);
                    }
                }
            }
        }
    }
}
