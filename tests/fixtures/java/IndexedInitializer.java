public class IndexedInitializer {
    static int consume(byte[] values) {
        int result = 0;
        for (int index = 0; index < values.length; index++) result = result * 31 + values[index];
        return result;
    }

    static int retainedIndex(byte[] data, int index) {
        int result = consume(new byte[] { data[index++], data[index++], data[index++], data[index++] });
        return result + index * 17 + data[index];
    }

    static int retainedCalculation(byte[] data, int index) {
        byte[] values = new byte[] { data[index++], data[index++], data[index++], data[index++] };
        int next = index + 1;
        return consume(values) + next * 17 + data[next];
    }

    static int exceptionalIndex(byte[] data, int index) {
        try {
            return consume(new byte[] { data[index++], data[index++], data[index++], data[index++] });
        } catch (ArrayIndexOutOfBoundsException failure) {
            return -index;
        }
    }

    public static void main(String[] args) {
        byte[] data = new byte[] { 2, 3, 5, 7, 11, 13, 17, 19 };
        for (int start = 0; start < 3; start++) {
            System.out.println(retainedIndex(data, start));
            System.out.println(retainedCalculation(data, start));
        }
        for (int start = 4; start < 9; start++) System.out.println(exceptionalIndex(data, start));
    }
}
