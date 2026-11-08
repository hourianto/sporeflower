public final class QualityLoops {
    private static int trace;

    private static int mark(int value) {
        trace = trace * 31 + value;
        return value;
    }

    public static void drawOnce(int[] frames) {
        for (int unused = 0; unused >= 0; unused--) {
            for (int frame = frames.length - 1; frame >= 0; frame--) {
                mark(frames[frame]);
            }
        }
        mark(11);
    }

    public static void ascendingOnce() {
        for (int unused = 4; unused < 5; unused++) {
            mark(12);
        }
    }

    public static void inclusiveOnce() {
        for (int unused = 30; unused <= 30; unused++) {
            mark(13);
        }
    }

    public static void strideOnce() {
        for (int unused = 7; unused == 7; unused += 9) {
            mark(14);
        }
    }

    public static void subtractOnce() {
        for (int unused = -2; unused > -3; unused -= 7) {
            mark(15);
        }
    }

    public static void wrappingOnce() {
        for (int unused = Integer.MAX_VALUE; unused >= 0; unused++) {
            mark(16);
        }
    }

    public static void zeroTrips() {
        for (int unused = 5; unused < 3; unused++) {
            mark(17);
        }
        mark(18);
    }

    public static int selectedOnce(boolean selected) {
        int result = 0;
        for (int unused = 1; true; unused++) {
            if (unused >= 2) {
                int finished = result;
                return finished;
            }
            if (selected) result = mark(44);
        }
    }

    public static int selectedZeroTrips(boolean selected) {
        int result = 0;
        for (int unused = 2; true; unused++) {
            if (unused >= 2) {
                int finished = result;
                return finished;
            }
            if (selected) result = mark(45);
        }
    }

    public static void emptyOnce() {
        for (int unused = -7; unused >= -7; unused--) {
        }
        mark(19);
    }

    public static void observedCounter(int[] values) {
        for (int index = 0; index < 1; index++) {
            values[index] += mark(index + 20);
        }
    }

    public static void modifiedCounter(int repeats) {
        for (int index = 0; index <= 0; index++) {
            mark(21);
            if (repeats-- > 0) {
                index--;
            }
        }
    }

    public static int escapingCounter() {
        int index;
        for (index = 1; index < 2; index++) {
            mark(22);
        }
        return index;
    }

    public static void effectfulInitializer() {
        for (int index = mark(0); index < 1; index++) {
            mark(23);
        }
    }

    public static void effectfulCondition() {
        for (int index = 0; mark(index) < 1; index++) {
            mark(24);
        }
    }

    public static void effectfulUpdate() {
        for (int index = 0; index < 1; index = mark(index + 1)) {
            mark(25);
        }
    }

    public static void ownedContinue(boolean skip) {
        for (int unused = 0; unused < 1; unused++) {
            mark(26);
            if (skip) {
                continue;
            }
            mark(27);
            mark(28);
        }
    }

    public static void ownedBreak(boolean stop) {
        for (int unused = 0; unused < 1; unused++) {
            mark(29);
            if (stop) {
                break;
            }
            mark(30);
        }
        mark(31);
    }

    public static void nestedTransfers(int[] values) {
        for (int unused = 5; unused != 8; unused += 3) {
            for (int index = 0; index < values.length; index++) {
                if (values[index] < 0) {
                    break;
                }
                if (values[index] == 0) {
                    continue;
                }
                mark(values[index]);
                mark(32);
            }
        }
    }

    public static void labelledContinue(int[] values) {
        outer: for (int unused = 0; unused < 1; unused++) {
            for (int index = 0; index < values.length; index++) {
                if (values[index] == 0) {
                    continue outer;
                }
                mark(values[index]);
            }
            mark(33);
        }
    }

    public static void protectedOnce(boolean fail) {
        for (int unused = 3; unused >= 3; unused--) {
            try {
                mark(34);
                if (fail) {
                    throw new IllegalArgumentException();
                }
            } catch (IllegalArgumentException failure) {
                mark(35);
            } finally {
                mark(36);
            }
        }
    }

    public static void twoTrips() {
        for (int unused = 0; unused < 2; unused++) {
            mark(37);
        }
    }

    public static void cleanupOverridesContinue(boolean selected) {
        for (int unused = 0; unused < 1; unused++) {
            if (selected) {
                try {
                    mark(39);
                    continue;
                } finally {
                    mark(40);
                    return;
                }
            }
            mark(41);
        }
    }

    public static void localScopesOnce() {
        for (int unused = 1; unused < 2; unused++) {
            int value = mark(42);
            mark(value);
        }
        {
            int value = mark(43);
            mark(value);
        }
    }

    public static void wrapDoesNotExit() {
        for (int index = Integer.MAX_VALUE; index != Integer.MIN_VALUE + 1; index++) {
            mark(38);
        }
    }

    public static void main(String[] args) {
        int[] values = new int[] { 3, 0, 4, -1 };
        drawOnce(values);
        ascendingOnce();
        inclusiveOnce();
        strideOnce();
        subtractOnce();
        wrappingOnce();
        zeroTrips();
        emptyOnce();
        observedCounter(values);
        modifiedCounter(2);
        mark(escapingCounter());
        effectfulInitializer();
        effectfulCondition();
        effectfulUpdate();
        for (int index = 0; index < 2; index++) {
            ownedContinue(index == 0);
            ownedBreak(index == 0);
            protectedOnce(index == 0);
            cleanupOverridesContinue(index == 0);
            mark(selectedOnce(index == 0));
            mark(selectedZeroTrips(index == 0));
        }
        nestedTransfers(values);
        labelledContinue(values);
        twoTrips();
        wrapDoesNotExit();
        localScopesOnce();
        System.out.println(trace);
        System.out.println(values[0]);
    }
}
