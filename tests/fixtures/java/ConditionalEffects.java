public class ConditionalEffects {
    static int token(int index) {
        System.out.println("read " + index);
        return index;
    }

    static Object parse(int token) {
        System.out.println("parse " + token);
        if (token == 1) throw new IllegalArgumentException("retry");
        return token == 0 ? null : new Object();
    }

    static void consume(int count) {
        Object saved = null;
        for (int index = 0; index < count; index++) {
            if (index == 4) saved = null;
            int token = token(index);
            try {
                // The result is only used by the next iteration's guard.
                saved = saved == null ? parse(token) : saved;
            } catch (IllegalArgumentException caught) {
                System.out.println(caught.getMessage());
            }
        }
    }

    public static void main(String[] args) {
        consume(0);
        consume(7);
    }
}
