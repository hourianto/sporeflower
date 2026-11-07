public final class SharedTailSupport {
    static StringBuffer trace;
    static int failAt;

    public static int mark(int value) {
        trace.append(value).append(',');
        if (failAt == value) throw new RuntimeException();
        return value;
    }

    public static int consume(int left, int right) {
        mark(40);
        return left * 31 + right;
    }

    public static void main(String[] args) {
        int[] failures = { 0, 1, 10, 11, 12, 13, 14, 20, 21, 23, 40, 90, 99 };
        for (int caught = 0; caught < 2; caught++) {
            for (int selector = -1; selector <= 6; selector++) {
                for (int guard = -1; guard <= 1; guard++) {
                    for (int failure = 0; failure < failures.length; failure++) {
                        trace = new StringBuffer();
                        failAt = failures[failure];
                        String result;
                        try {
                            int value = caught == 0
                                ? SharedTailCases.run(selector, guard)
                                : SharedTailCases.protectedRun(selector, guard);
                            result = "return:" + value;
                        } catch (RuntimeException ex) {
                            result = "throw";
                        }
                        System.out.println(caught + ":" + selector + ":" + guard + ":" + failAt + ":" + trace + result);
                    }
                }
            }
        }
    }
}
