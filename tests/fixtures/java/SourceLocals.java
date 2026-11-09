public class SourceLocals {
    static int trace;

    static int step(int value) {
        trace = trace * 31 + value;
        if (value == -1) throw new IllegalArgumentException("step");
        return value;
    }

    static int straight() {
        step(7);
        int value = step(3);
        return value * value;
    }

    static int loop(int count) {
        int total = 0;
        for (int i = 0; i < count; i++) {
            int value = step(i);
            total += value * value;
        }
        return total;
    }

    static int branches(boolean branch) {
        int result;
        if (branch) {
            int value = step(5);
            result = value * value;
        } else {
            int value = step(6);
            result = value * value;
        }
        return result;
    }

    static int delayedBranches(boolean branch) {
        step(19);
        int result;
        if (branch) {
            step(20);
            result = step(21);
        } else {
            step(22);
            result = step(23);
        }
        return result * result;
    }

    static int optionalLiteral(boolean assign, int input) {
        int result = 0;
        step(24);
        if (assign) {
            step(25);
            result = step(input);
        }
        return result * result;
    }

    static int singleUseLiteral(boolean assign, int input) {
        int result = 0;
        step(28);
        if (assign) {
            step(29);
            result = step(input);
        }
        return result;
    }

    static int optionalCall(boolean assign, int input) {
        int result = 0;
        if (assign) result = step(input);
        return result;
    }

    static int singleUseLoop(boolean assign, int input) {
        int result = -1;
        if (assign) {
            for (int i = 0; i < input; i++) step(i);
            result = step(input);
        }
        return result;
    }

    static int singleUseNested(boolean assign, int input) {
        int result = -1;
        if (assign) {
            if (input > 0) step(32);
            result = step(input);
        }
        return result;
    }

    static int optionalNestedCall(boolean assign, int input) {
        int result = 0;
        if (assign) result = input > 0 ? step(input) : step(33);
        return result;
    }

    static int singleUseObserved(boolean assign, int input) {
        String result = null;
        boolean empty = !assign;
        if (!empty) {
            result = String.valueOf(step(input));
            empty = result.length() == 0;
        }
        return empty ? -1 : result.length();
    }

    static int singleUsePair(boolean assign, int input, int backup) {
        int result = 0;
        int other = backup;
        if (assign) {
            result = input;
            other = input;
        }
        return result + other;
    }

    static int singleUseSwitch(int selector, int input) {
        int result = 0;
        switch (selector) {
            case 0:
                step(30);
                result = step(input);
                break;
            case 1:
                step(31);
                result = 2;
                break;
        }
        return result;
    }

    static int repeatedLiteral(boolean assign, int count) {
        int total = 0;
        for (int i = 0; i < count; i++) {
            int current = 3;
            step(26);
            if (assign) {
                step(27);
                current = step(i);
            }
            total += current * current;
        }
        return total;
    }

    static int cursorIndex(int input, int count) {
        int cursor = 0;
        int sum = 0;
        for (int i = 0; i < count; i++) {
            sum += (i & 1) == 0 ? cursor++ : input * 2 + 1;
        }
        return cursor * 100 + sum;
    }

    static int guarded(int input) {
        int result = 13;
        try {
            int value = step(input);
            result = value * value;
        } catch (IllegalArgumentException failure) {
            result += failure.getMessage().length();
        }
        return result;
    }

    static long guardedLong(int input, boolean assign) {
        long result = 0L;
        step(12);
        try {
            if (assign) {
                result = step(input);
                result += step(input - 1);
            }
        } catch (IllegalArgumentException failure) {
            trace = trace * 31 + 13;
        } finally {
            trace = trace * 31 + (int) result;
        }
        return result;
    }

    static String guardedReference(int input) {
        String result = null;
        try {
            result = String.valueOf(step(input));
            step(input - 1);
        } catch (IllegalArgumentException failure) {
            trace = trace * 31 + 14;
        } finally {
            trace = trace * 31 + (result == null ? 15 : result.length());
        }
        return result;
    }

    static Object monitor(int input) {
        step(16);
        return input < 0 ? null : new Object();
    }

    static int guardedMonitor(int input) {
        int result = 17;
        try {
            synchronized (monitor(input)) {
                result = step(input);
            }
        } catch (RuntimeException failure) {
            trace = trace * 31 + 18;
        } finally {
            trace = trace * 31 + result;
        }
        return result;
    }

    static int switching(int input) {
        int result;
        switch (input) {
            case 0: {
                int value = step(8);
                result = value * value;
                break;
            }
            case 1: {
                int value = step(9);
                result = value * value;
                break;
            }
            default:
                result = step(10);
        }
        return result;
    }

    public static void main(String[] args) {
        System.out.println(straight());
        for (int i = -2; i < 7; i++) {
            System.out.println(loop(i));
            System.out.println(guarded(i));
            System.out.println(guardedLong(i, false));
            System.out.println(guardedLong(i, true));
            System.out.println(guardedReference(i));
            System.out.println(guardedMonitor(i));
            System.out.println(switching(i));
            System.out.println(branches(i % 2 == 0));
            System.out.println(delayedBranches(i % 2 == 0));
            System.out.println(optionalLiteral(false, i));
            System.out.println(optionalLiteral(true, i + 3));
            System.out.println(singleUseLiteral(false, i));
            System.out.println(singleUseLiteral(true, i + 3));
            System.out.println(optionalCall(false, i));
            System.out.println(optionalCall(true, i + 3));
            System.out.println(singleUseLoop(false, i));
            System.out.println(singleUseLoop(true, i + 2));
            System.out.println(singleUseNested(false, i));
            System.out.println(singleUseNested(true, i + 2));
            System.out.println(optionalNestedCall(false, i));
            System.out.println(optionalNestedCall(true, i));
            System.out.println(singleUseObserved(false, i));
            System.out.println(singleUseObserved(true, i + 3));
            System.out.println(singleUsePair(false, i, i + 1));
            System.out.println(singleUsePair(true, i, i + 1));
            System.out.println(singleUseSwitch(i, i + 3));
            System.out.println(repeatedLiteral(false, i));
            System.out.println(repeatedLiteral(true, i));
            System.out.println(cursorIndex(i, i + 3));
        }
        try {
            singleUseLiteral(true, -1);
        } catch (IllegalArgumentException failure) {
            System.out.println(failure.getMessage());
        }
        try {
            optionalCall(true, -1);
        } catch (IllegalArgumentException failure) {
            System.out.println(failure.getMessage());
        }
        try {
            singleUseLoop(true, -1);
        } catch (IllegalArgumentException failure) {
            System.out.println(failure.getMessage());
        }
        try {
            singleUseNested(true, -1);
        } catch (IllegalArgumentException failure) {
            System.out.println(failure.getMessage());
        }
        System.out.println(trace);
    }
}
