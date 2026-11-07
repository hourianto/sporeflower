public class BooleanEqualityDriver {
    public static void main(String[] args) {
        int[] values = { -5, 0, 1, 2, 76, Integer.MIN_VALUE, Integer.MAX_VALUE };
        for (int i = 0; i < values.length; i++) {
            for (int j = 0; j < values.length; j++) {
                System.out.println(BooleanEquality.equal(values[i], values[j]));
                System.out.println(BooleanEquality.different(values[i], values[j]));
            }
        }
    }
}
