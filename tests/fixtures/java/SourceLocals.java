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
            System.out.println(cursorIndex(i, i + 3));
        }
        System.out.println(trace);
    }
}
