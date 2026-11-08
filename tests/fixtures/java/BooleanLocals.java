public class BooleanLocals {
    static int trace;

    static boolean pairedFlags(int first, int second, int third, int fourth) {
        int empty = first == 0 ? 1 : 0;
        int assigned = second != -1 ? 1 : 0;
        int otherEmpty = third == 0 ? 1 : 0;
        int otherAssigned = fourth != -1 ? 1 : 0;
        return empty != otherEmpty || empty != 0 && assigned != otherAssigned;
    }

    static boolean equalityChain(int first, int second, int third) {
        int left = first < 0 ? 1 : 0;
        int middle = second < 0 ? 1 : 0;
        int right = third < 0 ? 1 : 0;
        return left == middle && middle != right || left == right;
    }

    static boolean loopFlags(int first, int second, int count) {
        int left = first < 0 ? 1 : 0;
        int right = second < 0 ? 1 : 0;
        for (int index = 0; index < count; index++) {
            left = left ^ right;
            right = index % 2 == 0 ? left : right;
        }
        return left != right || left != 0;
    }

    static boolean descriptorFlags(int first, int second) {
        boolean left = first < 0;
        boolean right = second < 0;
        consume(left);
        return left != right;
    }

    static boolean exactOne(int first) {
        int flag = first < 0 ? 1 : 0;
        consume(flag == 1);
        return flag == 1;
    }

    static boolean numericArguments(int first, int second) {
        int left = first < 0 ? 1 : 0;
        int right = second < 0 ? 1 : 0;
        consume(left);
        return left != right || left != 0;
    }

    static int numericReturn(int first, int second) {
        int left = first < 0 ? 1 : 0;
        int right = second < 0 ? 1 : 0;
        return left == right ? left : right;
    }

    static int numericIndex(int first, int second, int[] values) {
        int left = first < 0 ? 1 : 0;
        int right = second < 0 ? 1 : 0;
        if (left == right) {
            return values[left];
        }
        return values[right];
    }

    static int numericSwitch(int first, int second) {
        int left = first < 0 ? 1 : 0;
        int right = second < 0 ? 1 : 0;
        if (left == right) {
            switch (left) {
                case 0: return 7;
                case 1: return 11;
                default: return -1;
            }
        }
        return 23;
    }

    static void consume(int value) { trace = trace * 31 + value + 100; }
    static void consume(boolean value) { trace = trace * 31 + (value ? 3 : 5); }

    public static void main(String[] args) {
        int[] inputs = {Integer.MIN_VALUE, -1, 0, 1, Integer.MAX_VALUE};
        int[] values = {17, 29};
        for (int a = 0; a < inputs.length; a++) {
            for (int b = 0; b < inputs.length; b++) {
                for (int c = 0; c < inputs.length; c++) {
                    int first = inputs[a];
                    int second = inputs[b];
                    int third = inputs[c];
                    System.out.println(pairedFlags(first, second, third, first)
                        + ":" + equalityChain(first, second, third)
                        + ":" + loopFlags(first, second, c)
                        + ":" + descriptorFlags(first, second)
                        + ":" + exactOne(first)
                        + ":" + numericArguments(first, second)
                        + ":" + numericReturn(first, second)
                        + ":" + numericIndex(first, second, values)
                        + ":" + numericSwitch(first, second) + ":" + trace);
                }
            }
        }
    }
}
