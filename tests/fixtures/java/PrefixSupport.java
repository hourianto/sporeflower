public class PrefixSupport {
    private static int trace;
    private static int steps;
    private static int failAt;

    public static int fold(int value, int divisor) {
        trace = trace * 31 + value;
        steps++;
        if (steps == failAt) throw new IllegalStateException("fold" + steps);
        return value % divisor;
    }

    public static void main(String[] args) {
        int[] values = {-7001, -1001, -17, -1, 0, 1, 7, 1000};
        int[] divisors = {0, 1, 7};
        for (int i = 0; i < values.length; i++) {
            for (int j = 0; j < divisors.length; j++) {
                for (int failure = 0; failure < 3; failure++) {
                    trace = 0;
                    steps = 0;
                    failAt = failure;
                    try {
                        int result = PrefixLoop.run(values[i], divisors[j]);
                        System.out.println(result + ":" + trace + ":" + steps);
                    } catch (Throwable thrown) {
                        System.out.println(thrown.getClass().getName() + ":" + thrown.getMessage() + ":" + trace + ":" + steps);
                    }
                }
            }
        }
    }
}
