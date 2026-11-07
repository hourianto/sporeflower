import java.io.IOException;

public class ProtectedPrefix {
    static int trace;

    static Object freshMonitor() { return new Object(); }

    static int nested(Object outer, int count, int mode) {
        try {
            count++;
            synchronized (outer) {
                synchronized (freshMonitor()) {
                    effect(mode);
                }
            }
        } catch (IOException failure) {
            return count * 10 + 1;
        } catch (RuntimeException failure) {
            return count * 10 + 2;
        }
        return count * 10 + 3;
    }

    static Object effect(int mode) throws IOException {
        trace = trace * 31 + mode;
        if (mode == 1) throw new IOException();
        if (mode == 2) throw new IllegalArgumentException();
        return "value";
    }

    static int store(Object[] array, int count, int mode) {
        try {
            array[count++] = effect(mode);
        } catch (IOException failure) {
            return count * 10 + 1;
        } catch (RuntimeException failure) {
            return count * 10 + 2;
        }
        return count * 10 + 3;
    }

    static int terminal(int count, int mode) {
        try {
            count += 2;
            return count + effect(mode).hashCode();
        } catch (IOException failure) {
            return count * 10 + 1;
        } catch (RuntimeException failure) {
            return count * 10 + 2;
        }
    }

    static int dividing(Object[] array, int divisor, int mode) {
        int count = 5;
        try {
            count = 12 / divisor;
            array[count++] = effect(mode);
        } catch (IOException failure) {
            return count * 10 + 1;
        } catch (RuntimeException failure) {
            return count * 10 + 2;
        }
        return count * 10 + 3;
    }

    static int repeating(Object[] array, int count, int mode) {
        for (int iteration = 0; iteration < 3; iteration++) {
            try {
                array[count++] = effect(mode);
            } catch (IOException failure) {
                trace += count;
            } catch (RuntimeException failure) {
                trace -= count;
            }
        }
        return count;
    }

    public static void main(String[] arguments) {
        for (int mode = 0; mode < 3; mode++) {
            for (int count = -1; count < 3; count++) {
                System.out.println(store(new Object[2], count, mode));
                System.out.println(store(null, count, mode));
                System.out.println(terminal(count, mode));
                System.out.println(dividing(new Object[2], count, mode));
                System.out.println(repeating(new Object[2], count, mode));
                System.out.println(nested(new Object(), count, mode));
                System.out.println(nested(null, count, mode));
            }
        }
        System.out.println(trace);
    }
}
