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

    static byte[] sparse() {
        byte[] values = new byte[300];
        values[0] = (byte) mark(10);
        values[1] = 2;
        values[2] = 3;
        values[100] = 4;
        values[299] = 5;
        return values;
    }

    static byte[] defaults() {
        byte[] values = new byte[300];
        values[0] = 0;
        values[1] = 0;
        values[2] = 0;
        values[100] = 0;
        values[299] = 0;
        return values;
    }

    static int[][] compactRows() {
        int[][] rows = new int[2][];
        int[] first = new int[3];
        first[1] = mark(11);
        rows[0] = first;
        rows[1] = new int[] { mark(12), mark(13) };
        return rows;
    }

    static int[] dense() {
        return new int[] { 1, 2, 3, 4, 5, 6, 7, 8 };
    }

    static Object[] nulls() {
        return new Object[] { null, null, null };
    }

    static float[] zeroFloats() {
        return new float[] { 0.0f, 0.0f, 0.0f };
    }

    static int[] effectfulZeros() {
        return new int[] { mark(0), mark(0) };
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
        byte[] sparse = sparse();
        byte[] defaults = defaults();
        int[][] compact = compactRows();
        System.out.println(sparse.length + ":" + sparse[0] + ":" + sparse[1] + ":" + sparse[100] + ":" + sparse[298] + ":" + sparse[299]);
        System.out.println(defaults.length + ":" + defaults[0] + ":" + defaults[100] + ":" + defaults[299]);
        System.out.println(compact[0][0] + ":" + compact[0][1] + ":" + compact[0][2] + ":" + compact[1][1] + ":" + dense()[7]);
        System.out.println(nulls()[2] + ":" + Float.floatToIntBits(zeroFloats()[1]) + ":" + effectfulZeros()[1] + ":" + trace);
        // huge() is compiled on both sides, but intentionally never allocated.
    }
}
