public class NestedCatchScopes {
    static int trace;

    static void step(int current, int failure) {
        trace = trace * 10 + current;
        if (current == failure) throw new IllegalStateException();
    }

    static int run(int failure) {
        try {
            step(1, failure);
            try {
                step(2, failure);
            } catch (Exception first) {
                step(3, failure);
            }
            step(4, failure);
            try {
                step(5, failure);
            } catch (Exception second) {
                step(6, failure);
            }
            return trace;
        } catch (Exception outer) {
            return -trace;
        }
    }

    static int quiet(int failure) {
        try {
            step(1, failure);
            try {
                step(2, failure);
            } catch (Exception ignored) {
            }
            step(4, failure);
            return trace;
        } catch (Exception outer) {
            return -trace;
        }
    }

    public static void main(String[] args) {
        for (int failure = 0; failure <= 6; failure++) {
            trace = 0;
            System.out.println(run(failure));
            trace = 0;
            System.out.println(quiet(failure));
        }
    }
}
