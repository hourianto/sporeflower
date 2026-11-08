public class BooleanEqualityDriver {
    public static void main(String[] args) {
        int[] values = { -5, 0, 1, 2, 3, 76, Integer.MIN_VALUE, Integer.MAX_VALUE };
        for (int i = 0; i < values.length; i++) {
            for (int j = 0; j < values.length; j++) {
                System.out.println(BooleanEquality.equal(values[i], values[j]));
                System.out.println(BooleanEquality.different(values[i], values[j]));
                System.out.println(BooleanEquality.exactlyTrue(values[i], values[j]));
                System.out.println(BooleanEquality.notExactlyTrue(values[i], values[j]));
                System.out.println(BooleanEquality.reversedTrue(values[i], values[j]));
                System.out.println(BooleanEquality.normalizedTrue(values[i], values[j]));
                System.out.println(BooleanEquality.normalizedFalse(values[i], values[j]));
            }
        }
    }
}
