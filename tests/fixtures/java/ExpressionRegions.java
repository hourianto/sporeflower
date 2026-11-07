public class ExpressionRegions {
    static int trace;
    static int count = 3723456;
    static String label = "result";
    static String item(int n) { trace = trace * 31 + n; return "item" + n; }
    static int number(int n) { trace = trace * 31 + n; return n; }

    static String summary() {
        int time = count;
        int hours = time / 3600000;
        String text = new StringBuffer().append("").append(hours / 10).append(hours % 10).toString();
        int remainder = time % 3600000;
        int minutes = remainder / 60000;
        text = new StringBuffer().append(text).append(':').append(minutes / 10).append(minutes % 10).toString();
        int seconds = remainder % 60000 / 1000;
        text = new StringBuffer().append(text).append(':').append(seconds / 10).append(seconds % 10).toString();
        return new StringBuffer().append(item(1)).append("\n\n").append(label).append(text)
            .append('\n').append(item(2)).append('\n').append(item(3)).append('\n').append(item(4))
            .append('\n').append(item(5)).append('\n').append(item(6)).append('\n').append(item(7))
            .append('\n').append(item(8)).append('\n').append(item(9)).append('\n').append(item(10))
            .append('\n').append(item(11)).append('\n').append(item(12)).toString();
    }
    static String staged() {
        String first = item(1);
        int quotient = number(2) / number(3);
        String second = new StringBuffer().append(first).append(quotient).append(item(4)).toString();
        int remainder = number(5) % number(6);
        return new StringBuffer().append(second).append(remainder).append(item(7)).toString();
    }
    static String throwing(int denominator) {
        try {
            String first = item(1);
            int quotient = number(2) / denominator;
            return new StringBuffer().append(first).append(quotient).append(item(3)).toString();
        } catch (ArithmeticException failure) {
            return item(4);
        }
    }
    static String primitiveSeed(long value) {
        return new StringBuffer(String.valueOf(value / 1024L)).append("KB/").append(value % 1024L).toString();
    }
    static String stringSeed(String value) {
        return new StringBuffer(String.valueOf(value)).append(item(1)).toString();
    }
    static String charsSeed(char[] value) {
        return new StringBuffer(String.valueOf(value)).append(item(1)).toString();
    }
    static String objectSeed(Object value) {
        return new StringBuffer(String.valueOf(value)).append(item(1)).toString();
    }
    static String nullableSeed(String value) {
        return new StringBuffer(value).append(item(1)).toString();
    }
    static Object[] capturedChains(boolean flag, char[] chars, Object object) {
        return new Object[] {
            new StringBuffer().append(flag).toString(),
            new StringBuffer().append(chars).toString(),
            new StringBuffer().append(object).append(item(7)).toString()
        };
    }
    static Object[] sharedCapture() {
        String shared;
        return new Object[] { shared = item(8), shared };
    }
    static String selectedChain(boolean selected, int divisor) {
        return new StringBuffer().append(item(1)).append(label)
            .append(selected && number(2) / divisor > 0 ? item(3) : item(4)).toString();
    }
    static String selectedObject(Object object, boolean selected) {
        return new StringBuffer().append(object)
            .append(selected || number(2) > 0 ? item(3) : item(4)).toString();
    }
    static int selectedCatch(boolean selected, int divisor) {
        int saved = 8;
        try {
            return new StringBuffer().append(item(1))
                .append(selected && (saved = number(2)) / divisor > 0 ? item(3) : item(4)).length();
        } catch (ArithmeticException failure) {
            return saved;
        }
    }
    static int conditionalCopyAnd(boolean flag, int replacement) {
        int saved = 9;
        int choice = flag && (saved = replacement) > 0 ? 1 : 2;
        return choice * 100 + saved;
    }
    static boolean conjunction(boolean flag, int left, int right) {
        return flag && left == right;
    }
    static boolean disjunction(boolean flag, int left, int right) {
        return flag || left == right;
    }
    static int conditionalCopyOr(boolean flag, int replacement) {
        int saved = 9;
        int choice = flag || (saved = replacement) > 0 ? 1 : 2;
        return choice * 100 + saved;
    }
    static int conditionalCopyReturn(boolean flag, int replacement) {
        int saved = 9;
        return (flag && (saved = replacement) > 0 ? saved : saved) + saved;
    }
    static int conditionalCopyCaught(boolean flag, int replacement, int divisor) {
        int saved = 9;
        try {
            int choice = flag && (saved = replacement) / divisor > 0 ? 1 : 2;
            return choice * 100 + saved;
        } catch (ArithmeticException failure) {
            return 300 + saved;
        }
    }
    static int[] indexedArray(int[] values, int marker) {
        number(marker);
        return values;
    }
    static int indexed(int[] values, int index) {
        return indexedArray(values, 1)[index++] + indexedArray(values, 2)[index++] + index;
    }
    static int indexedSaved(int[] values, int index) {
        int saved = index;
        return values[index++] + values[index++] + saved;
    }
    static int indexedDescending(int[] values, int index) {
        return values[index--] + values[index--] + index;
    }
    static int indexedShared(int[] values, int index) {
        int saved = index++;
        return values[saved] + values[saved] + index;
    }
    static int indexedCaught(int[] values, int index) {
        try {
            return values[index++] + values[index++] + index;
        } catch (RuntimeException failure) {
            return index;
        }
    }
    static int indexedChoice(int[] values, int index, boolean choice) {
        return choice ? values[index++] + index : index;
    }
    static int indexedLoop(byte[] values, int index, int count) {
        int sum = 0;
        while (count >= 4) {
            sum += values[index++];
            sum += values[index++];
            sum += values[index++];
            sum += values[index++];
            count -= 4;
        }
        return sum * 100 + index;
    }
    static int indexedWrites(int value, byte[] values, int index) {
        values[index++] = (byte) (value >> 24);
        values[index++] = (byte) (value >> 16);
        values[index++] = (byte) (value >> 8);
        values[index++] = (byte) value;
        return index;
    }
    static String found(int[] values, int wanted) {
        int flag = 0;
        for (int i = 0; i < values.length; i++) {
            if (values[i] == wanted) { flag = 1; break; }
        }
        if (flag != 0) return "found";
        return "missing";
    }
    static int numericFlag(boolean condition) {
        int flag = condition ? 1 : 0;
        if (flag != 0) trace++;
        return flag + 4;
    }
    static String noncanonical(int value) {
        int flag = value;
        if (value < 0) flag = 1;
        return flag == 1 ? "one" : "other";
    }
    static int[] repeated(Object value) {
        return (int[])(Object)(int[])(Object)(int[])value;
    }
    static Second crossed(Object value) {
        return (Second)(Object)(First)value;
    }
    static double floating(double value) {
        return value / 0.0 + value % 0.0;
    }
    static void ignoredTest(int[] values) {
        if (number(1) == 3) { }
        if (values[0] == 4) { }
        number(2);
    }
    static void ignoredGuardedTest(boolean selected, int[] values) {
        if (selected && number(3) != 4 && values[0] != 5) { }
        number(6);
    }
    static void mergedFlags(boolean[] flags, boolean[][] children, int kind) {
        boolean first = flags[0] ? true : false;
        boolean second = flags[1] ? true : false;
        if (kind == 15 || kind == 16) {
            for (int i = 0; i < children.length; i++) {
                if (children[i][0]) first = true;
                if (children[i][1]) second = true;
            }
        }
        String left = first ? item(6) : null;
        String right = second ? item(9) : null;
        number(left == null ? 3 : 4);
        number(right == null ? 7 : 8);
    }
    public static void main(String[] args) {
        trace = 0; System.out.println(summary()); System.out.println(trace);
        trace = 0; System.out.println(staged()); System.out.println(trace);
        trace = 0; System.out.println(throwing(0)); System.out.println(trace);
        trace = 0; System.out.println(throwing(2)); System.out.println(trace);
        System.out.println(primitiveSeed(1234567L));
        trace = 0; System.out.println(stringSeed(null)); System.out.println(trace);
        trace = 0; System.out.println(charsSeed(new char[] {'a', 'b'})); System.out.println(trace);
        trace = 0;
        try { charsSeed(null); } catch (NullPointerException expected) { System.out.println(trace); }
        try { objectSeed(new NullString()); } catch (NullPointerException expected) { System.out.println(trace); }
        try { nullableSeed(null); } catch (NullPointerException expected) { System.out.println(trace); }
        trace = 0;
        Object[] captured = capturedChains(true, new char[] {'a', 'b'}, new NullString());
        for (int i = 0; i < captured.length; i++) System.out.println(captured[i]);
        System.out.println(trace);
        trace = 0;
        try { capturedChains(false, null, new NullString()); }
        catch (NullPointerException expected) { System.out.println(trace); }
        trace = 0;
        Object[] shared = sharedCapture();
        System.out.println(shared[0] == shared[1]);
        System.out.println(trace);
        for (int selected = 0; selected < 2; selected++) {
            for (int divisor = 0; divisor < 3; divisor++) {
                trace = 0;
                try { System.out.println(selectedChain(selected != 0, divisor)); }
                catch (ArithmeticException expected) { System.out.println("division"); }
                System.out.println(trace);
                trace = 0;
                System.out.println(selectedCatch(selected != 0, divisor));
                System.out.println(trace);
            }
            trace = 0;
            System.out.println(selectedObject(new NullString(), selected != 0));
            System.out.println(trace);
            for (int replacement = -1; replacement <= 1; replacement++) {
                System.out.println(conjunction(selected != 0, replacement, 1));
                System.out.println(disjunction(selected != 0, replacement, 1));
                System.out.println(conditionalCopyAnd(selected != 0, replacement));
                System.out.println(conditionalCopyOr(selected != 0, replacement));
                System.out.println(conditionalCopyReturn(selected != 0, replacement));
                System.out.println(conditionalCopyCaught(selected != 0, replacement, 0));
                System.out.println(conditionalCopyCaught(selected != 0, replacement, 1));
            }
        }
        System.out.println(floating(1.0));
        trace = 0; ignoredTest(new int[] {0}); System.out.println(trace);
        trace = 0;
        try { ignoredTest(null); } catch (NullPointerException expected) { System.out.println(trace); }
        trace = 0; ignoredGuardedTest(false, null); System.out.println(trace);
        trace = 0;
        try { ignoredGuardedTest(true, null); } catch (NullPointerException expected) { System.out.println(trace); }
        for (int bits = 0; bits < 16; bits++) {
            for (int kind = 14; kind <= 16; kind++) {
                trace = 0;
                mergedFlags(new boolean[] { (bits & 1) != 0, (bits & 2) != 0 },
                    new boolean[][] { { (bits & 4) != 0, (bits & 8) != 0 }, { false, false } }, kind);
                System.out.println(trace);
            }
        }
        int[] array = new int[] {17};
        int[] indexes = new int[] { -2147483648, -1, 0, 1, 2, 2147483647 };
        for (int i = 0; i < indexes.length; i++) {
            int index = indexes[i];
            int[] values = new int[] { 7, 11, 19 };
            trace = 0;
            try { System.out.println(indexed(values, index)); }
            catch (RuntimeException failure) { System.out.println(failure.getClass().getName()); }
            System.out.println(trace);
            try { System.out.println(indexedDescending(values, index)); }
            catch (RuntimeException failure) { System.out.println(failure.getClass().getName()); }
            try { System.out.println(indexedSaved(values, index)); }
            catch (RuntimeException failure) { System.out.println(failure.getClass().getName()); }
            try { System.out.println(indexedShared(values, index)); }
            catch (RuntimeException failure) { System.out.println(failure.getClass().getName()); }
            System.out.println(indexedCaught(values, index));
            System.out.println(indexedCaught(null, index));
            System.out.println(indexedChoice(null, index, false));
        }
        trace = 0;
        try { indexed(null, 0); }
        catch (RuntimeException failure) { System.out.println(failure.getClass().getName()); }
        System.out.println(trace);
        byte[] bytes = new byte[] { 1, 3, 5, 7, 9, 11, 13, 15 };
        System.out.println(indexedLoop(bytes, 0, 8));
        System.out.println(indexedWrites(-123456789, bytes, 2));
        for (int i = 0; i < bytes.length; i++) System.out.println(bytes[i]);
        System.out.println(found(array, 17));
        System.out.println(found(array, 18));
        System.out.println(found(new int[0], 17));
        System.out.println(numericFlag(true));
        System.out.println(numericFlag(false));
        for (int i = -1; i <= 2; i++) System.out.println(noncanonical(i));
        System.out.println(repeated(array) == array);
        System.out.println(repeated(null) == null);
        try { repeated(new Object()); } catch (ClassCastException expected) { System.out.println("array cast"); }
        try { crossed(new SecondOnly()); } catch (ClassCastException expected) { System.out.println("first cast"); }
        try { crossed(new FirstOnly()); } catch (ClassCastException expected) { System.out.println("second cast"); }
        System.out.println(crossed(new Both()) instanceof Both);
    }
}
class NullString {
    public String toString() { ExpressionRegions.trace = ExpressionRegions.trace * 31 + 2; return null; }
}

interface First {}
interface Second {}
class FirstOnly implements First {}
class SecondOnly implements Second {}
class Both implements First, Second {}
