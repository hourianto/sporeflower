public class IntegralIdioms {
    static int trace;
    static final int ALL = -1;

    static int step(int value) { trace = trace * 31 + value; return value; }
    static int complement(int value) { return value ^ -1; }
    static int complementLeft(int value) { return -1 ^ value; }
    static long complementLong(long value) { return value ^ -1L; }
    static long complementWidened(int value) { return value ^ -1L; }
    static int complementByte(byte value) { return value ^ -1; }
    static int complementChar(char value) { return value ^ -1; }
    static int complementSum(int left, int right) { return (step(left) + step(right)) ^ -1; }
    static int doubleComplement(int value) { return (value ^ -1) ^ -1; }
    static long partialLongMask(long value) { return value ^ 4294967295L; }
    static int partialIntMask(int value) { return value ^ 255; }
    static int mapped(int value) { return value ^ ALL; }
    static boolean boolXor(boolean left, boolean right) { return left ^ right; }
    static int identities(int value) { return (step(value) & -1) | 0; }
    static long wideIdentity(int value) { return value ^ 0L; }
    static int eager(int value) { return (step(value) & 0) | (step(value + 1) ^ 0); }
    static int throwingIdentity(int value) { return (7 / value) & -1; }
    static int width(byte value) { return 1; }
    static int width(int value) { return 2; }
    static int width(long value) { return 3; }
    static int identityWidths(byte narrow, int value) {
        return width(narrow & -1) * 100 + width(value ^ 0L) * 10 + width(value ^ -1L);
    }

    static int zeroMinus(int value) { return 0 - value; }
    static long zeroMinusWidened(int value) { return 0L - value; }
    static int subtractNegated(int left, int right) { return step(left) - -step(right); }
    static int addNegated(int left, int right) { return step(left) + -step(right); }
    static int doubleNegated(int value) { return -(-value); }
    static long widenedNegation(int value) { return -((long) -value); }
    static long subtractNarrowNegation(long left, int right) { return left - -right; }
    static float floatingZeroMinus(float value) { return 0.0F - value; }

    public static void main(String[] arguments) {
        int[] values = { Integer.MIN_VALUE, -1, 0, 1, Integer.MAX_VALUE };
        for (int i = 0; i < values.length; i++) {
            int value = values[i];
            System.out.println(complement(value) + ":" + complementLeft(value) + ":" + complementLong(value) + ":" + complementWidened(value));
            System.out.println(complementByte((byte) value) + ":" + complementChar((char) value) + ":" + complementSum(value, value + 1) + ":" + doubleComplement(value));
            System.out.println(partialLongMask(value) + ":" + partialIntMask(value) + ":" + mapped(value) + ":" + identities(value) + ":" + wideIdentity(value) + ":" + eager(value));
            System.out.println(zeroMinus(value) + ":" + zeroMinusWidened(value) + ":" + subtractNegated(value, value) + ":" + addNegated(value, value));
            System.out.println(doubleNegated(value) + ":" + widenedNegation(value) + ":" + subtractNarrowNegation(value, value));
            System.out.println(identityWidths((byte) value, value));
            try { System.out.println(throwingIdentity(value)); }
            catch (ArithmeticException exception) { System.out.println("zero"); }
        }
        long[] wide = { Long.MIN_VALUE, -4294967296L, -1L, 0L, 4294967296L, Long.MAX_VALUE };
        for (int i = 0; i < wide.length; i++) System.out.println(complementLong(wide[i]) + ":" + partialLongMask(wide[i]));
        System.out.println(boolXor(false, false) + ":" + boolXor(false, true) + ":" + boolXor(true, false) + ":" + boolXor(true, true));
        System.out.println(Float.floatToIntBits(floatingZeroMinus(0.0F)) + ":" + Float.floatToIntBits(floatingZeroMinus(-0.0F)) + ":" + Float.floatToIntBits(floatingZeroMinus(Float.NaN)));
        System.out.println(trace);
    }
}
