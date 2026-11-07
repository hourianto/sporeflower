public class StringConcatenation {
    static int effects;
    static final RuntimeException failure = new RuntimeException("same");

    static String next() { effects = effects * 10 + 2; return "next"; }
    static int capacity() { effects = effects * 10 + 3; return 4; }
    static String warning(char value) {
        return new StringBuffer().append("symbol ").append(value).append(" (").append((int)value).append(")").toString();
    }
    static String numbers(int a, int b) {
        return new StringBuffer().append(a).append(b).append(a + b).toString();
    }
    static String floating(float value) {
        return new StringBuffer().append(value).append(':').append((double)value).toString();
    }
    static String objectFirst(Object value) {
        return new StringBuffer().append(value).append(next()).toString();
    }
    static void discarded(Object value) { new StringBuffer().append(value).append(next()).toString(); }
    static String constant() {
        return new StringBuffer().append("a").append("bc").toString();
    }
    static String single(String value) { return new StringBuffer().append(value).toString(); }
    static String prefix(int value) { return new StringBuffer("prefix:").append(value).toString(); }
    static String stringConstructor(String value) {
        return new StringBuffer(value).append(next()).toString();
    }
    static String negativeCapacity() { return new StringBuffer(-1).append(next()).toString(); }
    static String dynamicCapacity() { return new StringBuffer(capacity()).append(next()).toString(); }
    static String characters(char[] value) { return new StringBuffer().append(value).append(next()).toString(); }
    static String objectArray(char[] value) { return new StringBuffer().append((Object)value).append('!').toString(); }
    static String shared() {
        StringBuffer buffer = new StringBuffer();
        String first = buffer.append("a").append("b").toString();
        buffer.append("c");
        return new StringBuffer().append(first).append(':').append(buffer.toString()).toString();
    }
    static int find(String text) { return text.indexOf(47) + text.lastIndexOf(58, 47); }

    public static void main(String[] args) {
        System.out.println(warning('\u00e9'));
        System.out.println(numbers(1, 2));
        System.out.println(floating(0.1f));
        effects = 0;
        System.out.println(objectFirst(new ConcatObject(0)));
        System.out.println(effects);
        effects = 0;
        discarded(new ConcatObject(0));
        System.out.println(effects);
        effects = 0;
        try { objectFirst(new ConcatObject(1)); }
        catch (RuntimeException value) { System.out.println(value == failure); }
        System.out.println(effects);
        effects = 0;
        System.out.println(objectFirst(new ConcatObject(2)));
        System.out.println(effects);
        System.out.println(constant() == "abc");
        System.out.println(constant() == constant());
        System.out.println(single("abc") == "abc");
        System.out.println(single(null));
        System.out.println(prefix(7));
        effects = 0;
        try { stringConstructor(null); }
        catch (NullPointerException value) { System.out.println(effects); }
        try { negativeCapacity(); }
        catch (NegativeArraySizeException value) { System.out.println(effects); }
        System.out.println(dynamicCapacity());
        System.out.println(effects);
        effects = 0;
        System.out.println(characters(new char[] { 'a', 'b' }));
        System.out.println(effects);
        effects = 0;
        try { characters(null); }
        catch (NullPointerException value) { System.out.println(effects); }
        char[] chars = new char[] { 'a', 'b' };
        System.out.println(objectArray(chars).equals(new StringBuffer().append(chars.toString()).append('!').toString()));
        System.out.println(shared());
        System.out.println(find("a/b:c"));
    }
}

class ConcatObject {
    final int mode;
    ConcatObject(int mode) { this.mode = mode; }
    public String toString() {
        StringConcatenation.effects = StringConcatenation.effects * 10 + 1;
        if (mode == 1) throw StringConcatenation.failure;
        if (mode == 2) return null;
        return "object";
    }
}
