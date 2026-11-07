public class CastContexts {
    static int choices;
    static int context(Object[] values, int index, Object fallback) { return 1; }
    static int context(StringBuffer value, int index, String suffix) { return 2; }
    static int objectOnly(Object value) { return 3; }
    static int objectOnly(int value) { return 4; }
    static int pair(Object first, Object second) { return 5; }
    static int pair(String first, String second) { return 6; }
    static int nullable(Object value) { return 7; }
    static int nullable(String value) { return 8; }
    static int ambiguous(Object value) { return 9; }
    static int ambiguous(String value) { return 10; }
    static int ambiguous(char[] value) { return 11; }
    static int numeric(int value) { return 12; }
    static int numeric(long value) { return 13; }
    static int numeric(float value) { return 14; }
    static int numeric(double value) { return 15; }
    static int numeric(byte value) { return 24; }
    static int numeric(short value) { return 25; }
    static int numeric(char value) { return 26; }
    static int onlyInt(int value) { return 27; }
    static int onlyLong(long value) { return 16; }
    static int onlyLong(String value) { return 17; }
    static int conditional(long value) { return 22; }
    static int conditional(char value) { return 23; }

    static int noConflict() { return context(new Object[0], 0, (Object)"text"); }
    static int objectCase() { return objectOnly((Object)"text"); }
    static int forcedPair() { return pair((Object)"left", (Object)"right"); }
    static int nullSpecific() { return nullable(null); }
    static int nullGeneral() { return nullable((Object)null); }
    static int nullAmbiguous() { return ambiguous((String)null); }
    static int wideningCall(int value) { return onlyLong((long)value); }
    static int forcedLong(int value) { return numeric((long)value); }
    static int forcedDouble(float value) { return numeric((double)value); }
    static int fromConditional(boolean flag, char value) { return conditional((long)(flag ? value : 1)); }
    static int inherited(CastChild value) { return ((CastBase)value).select((Object)"text"); }
    static boolean objectCharacters(char[] value) {
        return String.valueOf((Object)value).equals(value == null ? "null" : value.toString());
    }
    static String stringValue(String value) { return String.valueOf((Object)value); }
    static long product(int left, int right) { return (long)left * (long)right; }
    static long widenedSum(int left, int right) { return (long)left + (long)right; }
    static long literalTimes(int value) { return 6553L * (long)value; }
    static long narrowThenWiden(long value, long factor) { return (long)(int)value * factor; }
    static long shift(int value) { return (long)value << 40; }
    static double divide(int left, int right) { return (double)left / (double)right; }
    static double rounded(long value) { return (double)(float)value; }
    static double roundedMixed(long value, double amount) { return (double)(float)value + amount; }
    static double additionOverflow(int left, int right) { return (double)(left + right); }
    static boolean compare(long value, double other) { return (double)(float)value < other; }
    static int narrowed(float value) { return (int)value; }

    static String nullChoice(boolean flag, String value) { return flag ? value : null; }
    static byte[] nullArrayChoice(boolean flag, byte[] value) { return flag ? null : value; }
    static int nullChoiceOverload(boolean flag, String value) { return nullable((Object)(flag ? value : null)); }

    static int byteFromZero(int count) {
        byte value = 0;
        int trace = 0;
        for (int index = 0; index < count; index++) {
            trace = trace * 31 + value + numeric((int)value);
            value++;
            if (index % 5 == 0) value += 25;
        }
        return trace * 31 + value;
    }

    static int narrowStart(int start) {
        byte value = (byte)start;
        int trace = value;
        for (int index = 0; index < 4; index++) {
            value++;
            trace = trace * 31 + value;
        }
        return trace;
    }

    static int smallByte(boolean flag) {
        byte value;
        if (flag) value = -128;
        else value = 127;
        return numeric(value) * 1000 + value;
    }

    static int mixedWrites(int replacement) {
        int value = 0;
        int trace = 0;
        for (int index = 0; index < 4; index++) {
            if (index == 1) value = replacement;
            else value = (byte)(value + 1);
            trace = trace * 31 + value;
        }
        return trace;
    }

    static int shortFromZero(int count) {
        short value = 0;
        int trace = 0;
        for (int index = 0; index < count; index++) {
            value += 32767;
            trace = trace * 31 + value;
        }
        return trace;
    }

    static int characterFromZero(int count) {
        char value = 0;
        int trace = 0;
        for (int index = 0; index < count; index++) {
            value += 65535;
            trace = trace * 31 + value;
        }
        return trace;
    }

    static int characterSwitch(char value) {
        switch (value) {
            case ' ': return 1;
            case '\'': return 2;
            case '\\': return 3;
            case '\n': return 4;
            case '\uabcd': return 5;
            default: return 0;
        }
    }

    static int integerSwitch(int value) {
        switch (value) {
            case 32: return 1;
            case -1: return 2;
            case 65536: return 3;
            default: return 0;
        }
    }

    static int narrowSwitch(int count) {
        int value = 0;
        for (int index = 0; index < count; index++) value = (byte)(value + 1);
        switch (value) {
            case -129: return 1;
            case 128: return 2;
            case 127: return 3;
            default: return value;
        }
    }

    static int byteChoice(boolean flag, int count) {
        int value = 0;
        for (int index = 0; index < count; index++) value = (byte)(value + 1);
        return numeric(flag ? value : 0);
    }

    static int conditionalNarrowSwitch(boolean flag, int count) {
        int value = 0;
        for (int index = 0; index < count; index++) value = (byte)(value + 1);
        switch (flag ? value : 0) {
            case -129: return 1;
            case 128: return 2;
            case 127: return 3;
            default: return -1;
        }
    }

    static int shortChoice(boolean flag, int count) {
        int value = 0;
        for (int index = 0; index < count; index++) value = (short)(value + 32767);
        return numeric(flag ? value : 0);
    }

    static int charChoice(boolean flag, int count) {
        int value = 0;
        for (int index = 0; index < count; index++) value = (char)(value + 65535);
        return numeric(flag ? value : 0);
    }

    static int nestedChoice(boolean flag, boolean other, int count) {
        int value = 0;
        for (int index = 0; index < count; index++) value = (byte)(value + 1);
        return numeric(flag ? (other ? value : 1) : 0);
    }

    static int assignedChoice(boolean flag, int count) {
        int value = 0;
        for (int index = 0; index < count; index++) value = (byte)(value + 1);
        int captured;
        int result = numeric(captured = flag ? value : 0);
        return result * 1000 + captured;
    }

    static int choiceConstructor(boolean flag, int count) {
        int value = 0;
        for (int index = 0; index < count; index++) value = (byte)(value + 1);
        return new NumericConstructor(flag ? value : 0).tag;
    }

    static int nestedConstructor(boolean flag, boolean other, int count) {
        int value = 0;
        for (int index = 0; index < count; index++) value = (char)(value + 65535);
        return new NumericConstructor(flag ? (other ? value : 1) : 0).tag;
    }

    static int shortConstructor(boolean flag, int count) {
        int value = 0;
        for (int index = 0; index < count; index++) value = (short)(value + 32767);
        return new NumericConstructor(flag ? value : 0).tag;
    }

    static int choiceWithoutOverload(boolean flag, int count) {
        int value = 0;
        for (int index = 0; index < count; index++) value = (byte)(value + 1);
        return onlyInt(flag ? value : 0);
    }

    static int nullStringChoice(boolean flag, String value) { return nullable(flag ? value : null); }
    static int nullBothChoice(boolean flag) { return nullable((Object)(flag ? null : null)); }
    static boolean nextChoice() { choices++; return choices % 2 == 0; }
    static int nullChoiceReceiver() {
        try { return (nextChoice() ? (String)null : (String)null).length(); }
        catch (NullPointerException failure) { return choices; }
    }

    public static void main(String[] args) {
        System.out.println(noConflict());
        System.out.println(objectCase());
        System.out.println(forcedPair());
        System.out.println(nullSpecific());
        System.out.println(nullGeneral());
        System.out.println(nullAmbiguous());
        System.out.println(wideningCall(7));
        System.out.println(forcedLong(7));
        System.out.println(forcedDouble(0.1F));
        System.out.println(fromConditional(true, 'a'));
        System.out.println(fromConditional(false, 'a'));
        System.out.println(inherited(new CastChild()));
        System.out.println(new CastConstructor((Object)"text").value);
        System.out.println(objectCharacters(new char[] { 'a', 'b' }));
        System.out.println(objectCharacters(null));
        System.out.println(stringValue("text"));
        System.out.println(product(100000, 100000));
        System.out.println(widenedSum(2147483647, 2));
        System.out.println(literalTimes(1000000));
        System.out.println(narrowThenWiden(4294967297L, 3L));
        System.out.println(shift(3));
        System.out.println(divide(3, 2));
        System.out.println(rounded(16777217L));
        System.out.println(roundedMixed(16777217L, 0.25D));
        System.out.println(additionOverflow(2147483647, 2));
        System.out.println(compare(16777217L, 16777217.0D));
        System.out.println(compare(16777217L, 0.0D / 0.0D));
        System.out.println(narrowed(2147483648.0F));
        System.out.println(narrowed(0.0F / 0.0F));
        System.out.println(nullChoice(false, "text"));
        System.out.println(nullChoice(true, "text"));
        System.out.println(nullArrayChoice(true, new byte[0]) == null);
        System.out.println(nullChoiceOverload(false, "text"));
        System.out.println(nullChoiceOverload(true, "text"));
        System.out.println(smallByte(false));
        System.out.println(smallByte(true));
        for (int count = 0; count < 140; count += 13) {
            System.out.println(byteFromZero(count));
            System.out.println(shortFromZero(count));
            System.out.println(characterFromZero(count));
        }
        int[] boundaries = { -65537, -32769, -129, -128, -1, 0, 1, 127, 128, 32768, 65536, 1000000 };
        for (int index = 0; index < boundaries.length; index++) {
            System.out.println(narrowStart(boundaries[index]));
            System.out.println(mixedWrites(boundaries[index]));
        }
        char[] characters = { ' ', '\'', '\\', '\n', '\uabcd', 'x' };
        for (int index = 0; index < characters.length; index++) {
            System.out.println(characterSwitch(characters[index]));
            System.out.println(integerSwitch(characters[index]));
        }
        System.out.println(integerSwitch(-1));
        System.out.println(integerSwitch(65536));
        System.out.println(narrowSwitch(0));
        System.out.println(narrowSwitch(127));
        System.out.println(narrowSwitch(128));
        System.out.println(narrowSwitch(256));
        System.out.println(nullChoiceReceiver());
        System.out.println(nullChoiceReceiver());
        for (int count = 0; count < 140; count += 13) {
            for (int branch = 0; branch < 4; branch++) {
                boolean flag = (branch & 1) != 0;
                boolean other = (branch & 2) != 0;
                System.out.println(byteChoice(flag, count));
                System.out.println(conditionalNarrowSwitch(flag, count));
                System.out.println(shortChoice(flag, count));
                System.out.println(charChoice(flag, count));
                System.out.println(nestedChoice(flag, other, count));
                System.out.println(assignedChoice(flag, count));
                System.out.println(choiceConstructor(flag, count));
                System.out.println(nestedConstructor(flag, other, count));
                System.out.println(shortConstructor(flag, count));
                System.out.println(choiceWithoutOverload(flag, count));
                System.out.println(nullStringChoice(flag, "text"));
                System.out.println(nullBothChoice(flag));
            }
        }
    }
}

class CastBase {
    public int select(Object value) { return 18; }
}
class CastChild extends CastBase {
    public int select(String value) { return 19; }
}
class CastConstructor {
    int value;
    CastConstructor(Object value) { this.value = 20; }
    CastConstructor(String value) { this.value = 21; }
}

class NumericConstructor {
    int tag;
    NumericConstructor(int value) { tag = 30; }
    NumericConstructor(byte value) { tag = 31; }
    NumericConstructor(short value) { tag = 32; }
    NumericConstructor(char value) { tag = 33; }
}
