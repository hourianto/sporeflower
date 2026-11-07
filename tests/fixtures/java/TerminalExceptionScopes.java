public class TerminalExceptionScopes {
    static int trace;
    static int failure;

    static int effect(int value) {
        trace = trace * 31 + value;
        if (failure == value) {
            throw new IllegalArgumentException(Integer.toString(value));
        }
        return value;
    }

    static int fragments(int bits) {
        RuntimeException caught;
        shared: {
            try {
                effect(1);
                if ((bits & 1) != 0) return 11;
            } catch (RuntimeException problem) {
                caught = problem;
                break shared;
            }
            try {
                effect(2);
                if ((bits & 2) != 0) return 12;
            } catch (RuntimeException problem) {
                caught = problem;
                break shared;
            }
            try {
                effect(3);
            } catch (RuntimeException problem) {
                caught = problem;
                break shared;
            }
            effect(4);
            return 14;
        }
        effect(5);
        return caught.getMessage().length() + 20;
    }

    static int cleanup(int bits) {
        done: {
            try {
                try {
                    effect(1);
                    if ((bits & 1) != 0) break done;
                    effect(2);
                } catch (IllegalArgumentException problem) {
                    effect(3);
                    break done;
                }
            } finally {
                effect(4);
            }
        }
        return effect(5);
    }

    static int override(int bits) {
        done: {
            try {
                effect(1);
                return 31;
            } finally {
                if ((bits & 1) != 0) break done;
                effect(2);
            }
        }
        return effect(3);
    }

    static int outside(int bits) {
        done: {
            try {
                effect(1);
                if ((bits & 1) != 0) break done;
                effect(2);
            } catch (IllegalArgumentException problem) {
                effect(3);
            }
            effect(4);
        }
        return effect(5);
    }

    public static void main(String[] args) {
        for (int method = 0; method < 4; method++) {
            for (int bits = 0; bits < 4; bits++) {
                for (failure = 0; failure < 6; failure++) {
                    trace = 0;
                    int value;
                    try {
                        switch (method) {
                        case 0: value = fragments(bits); break;
                        case 1: value = cleanup(bits); break;
                        case 2: value = override(bits); break;
                        default: value = outside(bits); break;
                        }
                    } catch (IllegalArgumentException problem) {
                        value = -Integer.parseInt(problem.getMessage());
                    }
                    System.out.println(new StringBuffer().append(value).append(':').append(trace).toString());
                }
            }
        }
    }
}
