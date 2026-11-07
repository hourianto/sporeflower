import java.io.ByteArrayInputStream;
import java.io.InputStream;
import java.io.IOException;

public class IndexOwnership {
    static int trace;

    static int[] receiver(int[] values, boolean fail) {
        trace = trace * 31 + 7;
        if (fail) throw new IllegalStateException();
        return values;
    }

    static int item(int index) {
        trace = trace * 31 + index;
        return index;
    }

    static int observedDelay(int[] values, int index, boolean fail) {
        try {
            int old = index++;
            return receiver(values, fail)[old] + index;
        } catch (RuntimeException failure) {
            return index;
        }
    }

    static int protectedBranches(int[] values, int index, int limit) {
        try {
            while (index < limit) {
                values[index++] = item(index);
                if (index % 2 == 0) {
                    trace = trace * 31 + 3;
                    continue;
                }
                trace = trace * 31 + 5;
            }
            return index;
        } catch (RuntimeException failure) {
            return -1;
        }
    }

    static int readChunks(InputStream input, byte[] buffer, int count) throws IOException {
        int filled = 0;
        while (filled < count) {
            int value;
            do {
                value = input.read();
                if (value == -1) break;
                buffer[filled++] = (byte) value;
            } while (filled % 3 != 0 && filled < count);
            trace = trace * 31 + filled;
            if (value == -1) break;
        }
        return filled;
    }

    static int retainedOld(int[] values, int index) {
        int original = index;
        int sum = values[index++] + values[index++];
        return sum * 100 + original * 10 + index;
    }

    public static void main(String[] arguments) throws Exception {
        for (int index = -1; index < 5; index++) {
            for (int fail = 0; fail < 2; fail++) {
                trace = 0;
                System.out.println(observedDelay(new int[] {2, 4, 6}, index, fail != 0));
                System.out.println(trace);
                trace = 0;
                System.out.println(observedDelay(null, index, fail != 0));
                System.out.println(trace);
            }
            trace = 0;
            System.out.println(protectedBranches(new int[4], index, 4));
            System.out.println(trace);
            trace = 0;
            System.out.println(protectedBranches(null, index, 4));
            System.out.println(trace);
        }
        for (int count = 0; count < 8; count++) {
            byte[] buffer = new byte[8];
            trace = 0;
            System.out.println(readChunks(new ByteArrayInputStream(new byte[] {1, 2, 3, 4, 5}), buffer, count));
            System.out.println(trace);
            for (int i = 0; i < buffer.length; i++) System.out.println(buffer[i]);
        }
        System.out.println(retainedOld(new int[] {7, 11, 19}, 0));
    }
}
