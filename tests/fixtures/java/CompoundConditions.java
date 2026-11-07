public class CompoundConditions {
    private static int trace;
    private int key = 52;
    private boolean pressed;

    public void key(int code) {
        if (code == 50 || code == key) pressed = true;
    }

    private static boolean probe(int code, int input) {
        trace = trace * 31 + code;
        if (input == code + 10) throw new IllegalArgumentException("p" + code);
        return (input & (1 << (code - 1))) != 0;
    }

    public static int mixed(int input) {
        if (probe(1, input) || probe(2, input) && probe(3, input)) return 7;
        return 9;
    }

    public static int grouped(int input) {
        if ((probe(1, input) || probe(2, input)) && (probe(3, input) || probe(4, input))) return 11;
        return 13;
    }

    public static int orderedStatements(int input) {
        if (probe(1, input)) {
            trace = trace * 31 + 17;
            if (probe(2, input)) return 19;
        }
        return 23;
    }

    public static int guardedNull(String value) {
        if (value != null && value.length() != 0 && value.charAt(0) == 'x') return 29;
        return 31;
    }

    public static int sharedBody(int input) {
        int result = 0;
        for (int i = 0; i < 3; i++) {
            switch (i) {
            case 1:
                if (probe(1, input) || !probe(2, input)) {
                    trace = trace * 31 + 53;
                    result += 7;
                }
                switch (input & 1) {
                case 0: result += 3; break;
                default: result += 5;
                }
                break;
            default: result++;
            }
        }
        return result;
    }

    public static int categorical(int input) {
        int result;
        if (probe(1, input)) {
            trace += 71;
            result = 1;
        } else if (probe(2, input)) {
            trace += 73;
            result = 2;
        } else if (probe(3, input)) {
            trace += 79;
            result = 3;
        } else {
            trace += 83;
            result = 4;
        }
        trace *= 31;
        return result;
    }

    public static int nestedDecisionBody(int input) {
        int result = 0;
        switch (input >>> 5) {
        case 0:
            if (probe(1, input) || probe(2, input) && probe(3, input)) {
                trace = trace * 31 + 89;
                result = 7;
            } else if (probe(4, input)) {
                return -3;
            }
            if (probe(5, input)) result += 11;
            else result += 13;
            break;
        default:
            result = 17;
        }
        return result;
    }

    public static int sharedCaseTail(int input) {
        int result = 0;
        for (int i = 0; i < 4; i++) {
            if ((input & 1) == 0) result += 3;
            else result += 5;
            int extra = 1;
            switch ((input + i) & 3) {
            case 0:
                trace = trace * 31 + 59;
                extra = probe(1, input) ? 7 : 11;
            case 1:
                trace = trace * 31 + 61;
                result += extra;
                break;
            case 2:
                result += 13;
                break;
            default:
                result += 17;
            }
            if (probe(2, input)) result += 19;
            else result += 23;
        }
        return result;
    }

    public static void tailLabel(int input) {
        done: {
            for (int i = 0; i < 5; i++) {
                if (probe(1, input)) break done;
                if (i == 2) break;
                trace = trace * 31 + 41;
            }
            trace = trace * 31 + 43;
        }
    }

    public static void main(String[] args) {
        for (int input = 0; input < 64; input++) {
            for (int method = 0; method < 8; method++) {
                trace = 0;
                try {
                    int result;
                    if (method == 0) result = mixed(input);
                    else if (method == 1) result = grouped(input);
                    else if (method == 2) result = orderedStatements(input);
                    else if (method == 3) { tailLabel(input); result = 47; }
                    else if (method == 4) result = sharedBody(input);
                    else if (method == 5) result = sharedCaseTail(input);
                    else if (method == 6) result = categorical(input);
                    else result = nestedDecisionBody(input);
                    System.out.println(result + ":" + trace);
                } catch (Throwable failure) {
                    System.out.println(failure.getClass().getName() + ":" + failure.getMessage() + ":" + trace);
                }
            }
        }
        System.out.println(guardedNull(null));
        System.out.println(guardedNull(""));
        System.out.println(guardedNull("x"));
        System.out.println(guardedNull("y"));
        for (int code = 49; code < 54; code++) {
            CompoundConditions value = new CompoundConditions();
            value.key(code);
            System.out.println(value.pressed);
        }
    }
}
