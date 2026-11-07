import java.io.IOException;

class SourceIdiomsBase {
    static int total = 31;
}

public class SourceIdioms extends SourceIdiomsBase {
    static int total = 17;
    static int e = 23;
    static final int beforeInitialization;
    static final int initialized;
    static {
        beforeInitialization = SourceIdioms.initialized;
        initialized = 19;
    }
    int value;

    static boolean slash(String text) { return text.charAt(0) != 47; }
    static boolean character(char c) { return 47 <= c && c <= 65535; }
    static boolean outsideCharacter(char c) { return c < -1 || c == 65536; }
    static boolean integer(int c) { return c != 47; }

    static int addNegative(int value) { return value + -7; }
    static int subtractNegative(int value) { return value - -7; }
    static long addNegativeLong(long value) { return value + -7L; }
    static int minimum(int value) { return value + Integer.MIN_VALUE; }
    static long minimumLong(long value) { return value - Long.MIN_VALUE; }
    static float floating(float value) { return value + -1.0F; }
    static int update(int value) {
        for (int i = 0; i < 2; i++) value += -7;
        return value;
    }
    void updateField() { value += -7; }
    static void updateArray(int[] values) { values[0] += -7; }

    static int ownField() { return total; }
    static int shadowed(int total) { return SourceIdioms.total + total; }
    static int inheritedField() { return SourceIdiomsBase.total; }

    static int unnamedCatch(int number) {
        try {
            return 100 / number;
        } catch (ArithmeticException ignored) {
            return e;
        }
    }

    static int namedCatch(int number) {
        try {
            return 100 / number;
        } catch (ArithmeticException cause) {
            return cause.toString().length();
        }
    }

    static Throwable sharedCatch(int number) {
        Throwable result;
        try {
            if (number == 0) throw new IOException("io");
            throw new IllegalStateException("state");
        } catch (IOException cause) {
            result = cause;
        } catch (IllegalStateException cause) {
            result = cause;
        }
        return result;
    }

    public static void main(String[] arguments) {
        System.out.println(beforeInitialization + ":" + initialized);
        System.out.println(slash("/") + ":" + slash("a"));
        System.out.println(character('/') + ":" + character('\uffff') + ":" + outsideCharacter('\uffff') + ":" + integer(47));
        int[] inputs = { Integer.MIN_VALUE, -7, 0, 7, Integer.MAX_VALUE };
        for (int i = 0; i < inputs.length; i++) {
            int value = inputs[i];
            System.out.println(addNegative(value) + ":" + subtractNegative(value) + ":" + minimum(value) + ":" + update(value));
        }
        System.out.println(addNegativeLong(Long.MAX_VALUE) + ":" + minimumLong(Long.MIN_VALUE));
        System.out.println(Float.floatToIntBits(floating(-0.0F)) + ":" + Float.floatToIntBits(floating(Float.NaN)));
        SourceIdioms object = new SourceIdioms();
        object.updateField();
        updateArray(inputs);
        System.out.println(object.value + ":" + inputs[0] + ":" + ownField() + ":" + shadowed(5) + ":" + inheritedField());
        System.out.println(unnamedCatch(0) + ":" + namedCatch(0) + ":" + sharedCatch(0) + ":" + sharedCatch(1));
    }
}
