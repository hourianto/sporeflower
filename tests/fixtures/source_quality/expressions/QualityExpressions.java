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

    static byte small;

    static int[] nullArray() {
        int[] result = null;
        return result;
    }

    static String nullString() {
        String result = null;
        return result;
    }

    static int nullKind(Object value) { return 1; }
    static int nullKind(String value) { return 2; }

    static int nullOverload() {
        Object result = null;
        return nullKind(result);
    }

    static byte constantReturn() {
        byte value = 12;
        return value;
    }

    static byte switchLiteral(int selector) {
        byte value;
        switch (selector) {
        case 0: value = 12; break;
        case 1: value = -1; break;
        default: value = 7;
        }
        return value;
    }

    static byte constantStores(byte[] values) {
        byte value = 12;
        small = value;
        values[0] = value;
        return value;
    }

    static byte byteChoice(boolean choose, byte value) {
        byte result;
        if (choose) result = value;
        else result = -1;
        return result;
    }

    static int characterChoice(boolean choose, char first, char second) {
        int result;
        if (choose) result = first;
        else result = second;
        return result;
    }

    static int mixedChoice(boolean choose, byte first, char second) {
        int result;
        if (choose) result = first;
        else result = second;
        return result;
    }

    static int promotedNarrow(int value) {
        short narrow = (short) value;
        int widened = narrow;
        return widened + 1;
    }

    static short nestedNarrow(long value) {
        int lower = (int) value;
        return (short) lower;
    }

    static int narrowedThenWidened(int value) {
        byte narrow = (byte) value;
        return narrow;
    }

    static int numericKind(short value) { return 1; }
    static int numericKind(int value) { return 2; }

    static int widenedOverload(short value) {
        int widened = value;
        return numericKind(widened);
    }

    static String characterAsNumber(char value) {
        int widened = value;
        return "" + widened;
    }

    static String narrowAsCharacter(int value) {
        char character = (char) value;
        return "" + character;
    }

    static double roundedWidening(int value) {
        float rounded = value;
        return rounded;
    }
}
