public class UnicodeLiterals {
    static final char LETTER = 'Ж';
    static final char SURROGATE = '\ud800';

    static String translated() {
        return "Crème brûlée — Ελληνικά Русский العربية 中文 日本語 한글";
    }

    static String supplementary() {
        return "😀 𐐀 𝄞";
    }

    static String combining() {
        return "e\u0301";
    }

    static char accented() { return 'é'; }
    static char quoted() { return '\''; }
    static char backslash() { return '\\'; }
    static char control() { return '\000'; }

    static String controls() {
        return "\0007\001\007\b\t\n\013\f\r\037\177\200\237";
    }

    static String hidden() {
        return "\u00a0\u00ad\u200b\u2028\u2029\u202e\ue000\uffff\udb40\udc01\udb80\udc00\u0378\ud83f\udffe";
    }

    static String surrogates() {
        return "\ud800x\udc00\udbff\ud800\udc00\udfff";
    }

    static String escapeText() {
        return "\"'\\u000a|\\\\u0041|\\uuuu000d|\ud800\\u005c|\\\ud800";
    }

    static void units(String value) {
        for (int i = 0; i < value.length(); i++) {
            System.out.print((int) value.charAt(i));
            System.out.print(',');
        }
        System.out.println();
    }

    public static void main(String[] args) {
        units(translated());
        units(supplementary());
        units(combining());
        units(controls());
        units(hidden());
        units(surrogates());
        units(escapeText());
        System.out.println((int) LETTER);
        System.out.println((int) SURROGATE);
        System.out.println((int) accented());
        System.out.println((int) quoted());
        System.out.println((int) backslash());
        System.out.println((int) control());
    }
}
