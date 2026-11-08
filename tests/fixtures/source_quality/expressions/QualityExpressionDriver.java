/** Deterministic values, mutations, exceptions and call traces for the corpus. */
public class QualityExpressionDriver {
    static void reset() { QualityExpressions.trace = 1; }

    static void report(String label, String result) {
        System.out.println(label + ":" + result + ":" + QualityExpressions.trace);
    }

    static String array(int[] values) {
        StringBuffer text = new StringBuffer();
        for (int i = 0; i < values.length; i++) text.append(values[i]).append(',');
        return text.toString();
    }

    static String bytes(byte[] values) {
        StringBuffer text = new StringBuffer();
        for (int i = 0; i < values.length; i++) text.append(values[i]).append(',');
        return text.toString();
    }

    public static void main(String[] args) {
        report("typed-nulls", "" + (QualityExpressions.nullArray() == null)
            + ":" + (QualityExpressions.nullString() == null)
            + ":" + QualityExpressions.nullOverload());
        String[] paths = { null, "", "/a?", "abc" };
        for (int p = 0; p < paths.length; p++) {
            for (int offset = -1; offset <= 3; offset++) {
                report("path", "" + QualityExpressions.pathCharacter(paths[p], offset));
            }
        }
        for (int enabled = 0; enabled < 2; enabled++) {
            for (int first = 0; first < 2; first++) {
                for (int second = 2; second < 4; second++) {
                    reset();
                    report("lazy", "" + QualityExpressions.lazyPolicy(enabled != 0, first, second));
                    reset();
                    report("eager", "" + QualityExpressions.eagerPolicy(first, second));
                    reset();
                    report("selected", "" + QualityExpressions.selectedPolicy(enabled != 0, first, second));
                }
            }
            reset();
            report("status", QualityExpressions.statusLine("job", enabled != 0));
            reset();
            report("null-status", QualityExpressions.statusLine(null, enabled != 0));
            reset();
            report("reference", "" + QualityExpressions.referenceChoice(enabled != 0, null, "second"));
        }
        int[] boundaries = { Integer.MIN_VALUE, -1, 0, 1, 10, 11, 65536, Integer.MAX_VALUE };
        for (int i = 0; i < boundaries.length; i++) {
            int value = boundaries[i];
            reset();
            report("clamp", "" + QualityExpressions.clampWithEffects(value));
            reset();
            report("arithmetic", "" + QualityExpressions.integerArithmetic(value));
            report("wide", "" + QualityExpressions.wideProduct(value, 65538, 13L));
            for (int divisor = -1; divisor <= 1; divisor++) {
                report("ratio", "" + Double.doubleToLongBits(QualityExpressions.ratio(value, divisor)));
            }
            int[] values = { value, 17 };
            reset();
            report("array-compound", QualityExpressions.arrayCompound(values, 0) + ":" + array(values));
            QualityExpressions.shared.count = value;
            reset();
            report("field-compound", "" + QualityExpressions.fieldCompound(QualityExpressions.shared));
            reset();
            report("increments", QualityExpressions.prefixAndPostfix(values) + ":" + array(values)
                + ":" + QualityExpressions.shared.count);
            byte[] packet = new byte[6];
            reset();
            report("packet", QualityExpressions.writePacket(packet, 1, value) + ":" + bytes(packet));
            int[][] matrix = QualityExpressions.nestedInitializer(value);
            report("matrix", array(matrix[0]) + ":" + array(matrix[1]) + ":" + array(matrix[2]));
        }
        char[] characters = { 0, 9, 10, 64, 65, 90, 91, 92, 93, '\uffff' };
        for (int i = 0; i < characters.length; i++) {
            report("character", "" + QualityExpressions.characterLiterals(characters[i]));
        }
        long[] sizes = { Long.MIN_VALUE, -1025L, -1L, 0L, 1023L, 1024L, Long.MAX_VALUE };
        for (int i = 0; i < sizes.length; i++) report("storage", QualityExpressions.storageLabel(sizes[i]));
        report("overload-null", "" + QualityExpressions.requiredOverload(null));
        report("overload-text", "" + QualityExpressions.requiredOverload("text"));
        reset();
        report("initializer", array(QualityExpressions.effectfulInitializer()));
        reset();
        report("guard-null", "" + QualityExpressions.guardedLength(null));
        reset();
        report("guard-text", "" + QualityExpressions.guardedLength("abc"));

        reset();
        try { QualityExpressions.arrayCompound(null, 0); }
        catch (NullPointerException expected) { report("array-null", "caught"); }
        reset();
        try { QualityExpressions.arrayCompound(new int[0], 0); }
        catch (ArrayIndexOutOfBoundsException expected) { report("array-range", "caught"); }
        reset();
        try { QualityExpressions.fieldCompound(null); }
        catch (NullPointerException expected) { report("field-null", "caught"); }
        QualityExpressions.shared.count = 19;
        reset();
        try { QualityExpressions.prefixAndPostfix(null); }
        catch (NullPointerException expected) { report("increments-null", "" + QualityExpressions.shared.count); }
        reset();
        try { QualityExpressions.prefixAndPostfix(new int[0]); }
        catch (ArrayIndexOutOfBoundsException expected) { report("increments-range", "" + QualityExpressions.shared.count); }
        reset();
        byte[] shortPacket = new byte[3];
        try { QualityExpressions.writePacket(shortPacket, 1, 0x12345678); }
        catch (ArrayIndexOutOfBoundsException expected) { report("packet-short", bytes(shortPacket)); }
        reset();
        try { QualityExpressions.writePacket(null, 0, 0); }
        catch (NullPointerException expected) { report("packet-null", "caught"); }

        report("constant-return", "" + QualityExpressions.constantReturn());
        byte[] stored = new byte[1];
        report("constant-stores", QualityExpressions.constantStores(stored) + ":"
            + QualityExpressions.small + ":" + stored[0]);
        int[] narrowValues = { Integer.MIN_VALUE, -65537, -32769, -129, -128,
            -1, 0, 127, 128, 32767, 32768, 65535, 65536, 16777217, Integer.MAX_VALUE };
        for (int i = 0; i < narrowValues.length; i++) {
            int value = narrowValues[i];
            report("numeric", QualityExpressions.promotedNarrow(value) + ":"
                + QualityExpressions.switchLiteral(value) + ":"
                + QualityExpressions.nestedNarrow((long) value * 4294967297L) + ":"
                + QualityExpressions.narrowedThenWidened(value) + ":"
                + QualityExpressions.widenedOverload((short) value) + ":"
                + QualityExpressions.characterAsNumber((char) value) + ":"
                + QualityExpressions.narrowAsCharacter(value) + ":"
                + Double.doubleToLongBits(QualityExpressions.roundedWidening(value)));
            for (int choose = 0; choose < 2; choose++) {
                report("numeric-choice", QualityExpressions.byteChoice(choose != 0, (byte) value) + ":"
                    + QualityExpressions.characterChoice(choose != 0, (char) value, '\uffff') + ":"
                    + QualityExpressions.mixedChoice(choose != 0, (byte) value, '\uffff'));
            }
        }
    }
}
