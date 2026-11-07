/** Small, composed examples of expression output that is already readable. */
public class QualityExpressions {
    static long trace;
    static final QualityExpressions shared = new QualityExpressions();
    int count;

    static int number(int value) {
        trace = trace * 31 + value;
        return value;
    }

    static boolean probe(int value) {
        number(value);
        return (value & 1) != 0;
    }

    static int index() { number(9); return 0; }
    static QualityExpressions receiver() { number(7); return shared; }

    static boolean pathCharacter(String text, int offset) {
        return text != null && offset >= 0 && offset < text.length()
            && (text.charAt(offset) == '/' || text.charAt(offset) == '?');
    }

    static boolean lazyPolicy(boolean enabled, int first, int second) {
        return enabled && probe(first) || probe(second);
    }

    static boolean eagerPolicy(int first, int second) {
        return (probe(first) & probe(second)) ^ probe(3);
    }

    static boolean selectedPolicy(boolean enabled, int first, int second) {
        return enabled ? probe(first) && probe(second) : probe(first) || probe(second);
    }

    static int clampWithEffects(int value) {
        return value < 0 ? number(0) : value > 10 ? number(10) : number(value);
    }

    static String statusLine(String label, boolean ready) {
        return new StringBuffer().append(label).append(':').append(number(1))
            .append(ready ? "/ready" : "/waiting").append('!').toString();
    }

    static String storageLabel(long bytes) {
        return new StringBuffer(String.valueOf(bytes / 1024L)).append("KB+")
            .append(bytes % 1024L).append('B').toString();
    }

    static boolean characterLiterals(char value) {
        return value >= 65 && value <= 90 || value == 10 || value == 92;
    }

    static int integerArithmetic(int value) {
        return (value + -7) * (value - -3) + (value >>> 5);
    }

    static long wideProduct(int width, int height, long extra) {
        return (long) (width + 1) * (height - 1) + extra;
    }

    static double ratio(int numerator, int denominator) {
        return (double) (numerator + 1) / denominator;
    }

    static Object referenceChoice(boolean choose, String first, String second) {
        return choose ? first : second;
    }

    static int kind(Object value) { return value == null ? 11 : 12; }
    static int kind(String value) { return value == null ? 21 : 22; }

    static int requiredOverload(String text) {
        return kind((Object) text) + kind((Object) null);
    }

    static int arrayCompound(int[] values, int index) {
        values[index] += number(5);
        values[index] ^= number(3);
        return values[index];
    }

    static int fieldCompound(QualityExpressions target) {
        target.count -= number(5);
        target.count |= 1;
        return target.count;
    }

    static int prefixAndPostfix(int[] values) {
        return values[index()]++ + ++receiver().count;
    }

    static int writePacket(byte[] bytes, int offset, int value) {
        bytes[offset++] = (byte) (value >>> 24);
        bytes[offset++] = (byte) (value >>> 16);
        bytes[offset++] = (byte) (value >>> 8);
        bytes[offset++] = (byte) value;
        return offset;
    }

    static int[] effectfulInitializer() {
        return new int[] { number(1), number(2) + number(3), number(4) };
    }

    static int[][] nestedInitializer(int value) {
        return new int[][] { { value, value + 1 }, { value + 2 }, {} };
    }

    static int guardedLength(String text) {
        if (text == null) return -1;
        number(6);
        if (text == null) return -2;
        return text.length();
    }
}
