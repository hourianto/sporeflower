public final class ChoiceConsumerSupport {
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
        int[] failures = { 0, 1, 2, 10, 11, 20, 21, 40 };
        for (int kind = 0; kind < 4; kind++) {
            for (int side = 0; side < 2; side++) {
                for (int index = -1; index <= 1; index++) {
                    for (int missing = 0; missing < 3; missing++) {
                        for (int failure = 0; failure < failures.length; failure++) {
                            int[] left = missing == 1 ? null : new int[1];
                            int[] right = missing == 2 ? null : new int[1];
                            trace = new StringBuffer();
                            failAt = failures[failure];
                            String result;
                            try {
                                int value;
                                if (kind == 0) value = ChoiceConsumers.store(side != 0, left, right, index);
                                else if (kind == 1) value = ChoiceConsumers.protectedStore(side != 0, left, right, index);
                                else if (kind == 2) value = ChoiceConsumers.choose(side != 0, left, right, index);
                                else value = ChoiceConsumers.protectedChoose(side != 0, left, right, index);
                                result = "return:" + value;
                            } catch (RuntimeException ex) {
                                result = "throw:" + ex.getClass().getName();
                            }
                            System.out.println(kind + ":" + side + ":" + index + ":" + missing + ":" + failAt
                                + ":" + trace + result + ":" + (left == null ? -1 : left[0])
                                + ":" + (right == null ? -1 : right[0]));
                        }
                    }
                }
            }
        }
    }
}
