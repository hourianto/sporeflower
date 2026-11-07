public class SparseArrays {
    static int trace;
    static int[] escaped;
    static int mark(int value) { trace = trace * 31 + value; return value; }
    static int fail(boolean failure) { if (failure) throw new IllegalStateException(); return mark(7); }

    static int[][] nested() {
        int[][] result = new int[4][];
        int[] first = new int[12];
        first[2] = mark(1);
        first[9] = mark(2);
        result[0] = first;
        result[1] = new int[12];
        int[] last = new int[6];
        last[3] = mark(3);
        result[3] = last;
        return result;
    }

    static int[] ordered() {
        int[] values = new int[8];
        values[5] = mark(4);
        values[2] = mark(5);
        values[5] = mark(6);
        return values;
    }

    static int[] escape() {
        int[] values = new int[8];
        values[2] = mark(8);
        escaped = values;
        mark(escaped[2] + escaped[7]);
        values[7] = mark(9);
        return values;
    }

    static int partial(boolean failure) {
        int[] values = null;
        try {
            values = new int[8];
            values[2] = mark(6);
            values[6] = fail(failure);
        } catch (RuntimeException problem) {
            return values[2] + values[6];
        }
        return values[2] + values[6];
    }

    static Object[] cycle() {
        Object[] values = new Object[4];
        values[1] = "marker";
        values[3] = values;
        return values;
    }

    static int[] huge() {
        int[] values = new int[1000000000];
        values[900000000] = 1;
        return values;
    }

    public static void main(String[] args) {
        int[][] matrix = nested();
        System.out.println(matrix[0][0] + ":" + matrix[0][2] + ":" + matrix[1][9] + ":" + matrix[2] + ":" + matrix[3][3]);
        int[] values = ordered();
        System.out.println(values[0] + ":" + values[2] + ":" + values[5]);
        values = escape();
        System.out.println((values == escaped) + ":" + values[7]);
        System.out.println(partial(false) + ":" + partial(true));
        Object[] cycle = cycle();
        System.out.println((cycle[3] == cycle) + ":" + cycle[0]);
        float[] floats = new float[4]; floats[2] = -0.0f;
        double[] doubles = new double[4]; doubles[2] = -0.0;
        long[] longs = new long[4]; longs[2] = 9L;
        boolean[] flags = new boolean[4]; flags[2] = true;
        System.out.println(Float.floatToIntBits(floats[0]) + ":" + Float.floatToIntBits(floats[2]));
        System.out.println(Double.doubleToLongBits(doubles[0]) + ":" + Double.doubleToLongBits(doubles[2]));
        System.out.println(longs[0] + ":" + longs[2] + ":" + flags[0] + ":" + flags[2] + ":" + trace);
        // huge() is compiled on both sides, but intentionally never allocated.
    }
}
