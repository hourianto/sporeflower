public class ConstantChoiceContexts {
    static String a = "x";
    static String b = "x";
    static String c = "x";
    static String d = "x";
    static final String constant = "x";
    static int mutable;

    static int pick(byte value) { return 10; }
    static int pick(int value) { return 20; }
    static int only(int value) { return 30; }

    static int literal(boolean choice) {
        return pick(choice ? new byte[] { 3 }[0] : a == b ? 0 : 1);
    }

    static int finalFields(boolean choice) {
        return pick(choice ? new byte[] { 4 }[0] : c == d ? 0 : 1);
    }

    static int concatenation(boolean choice) {
        return pick(choice ? new byte[] { 5 }[0] : new StringBuffer().append(a).append(b).toString() == "xx" ? 0 : 1);
    }

    static int noOverload(boolean choice) {
        return only(choice ? new byte[] { 6 }[0] : a == b ? 0 : 1);
    }

    static int interfaceField(boolean choice) {
        return pick(choice ? new byte[] { 7 }[0] : RawConstantChoice.value);
    }

    static int mutableField(boolean choice) {
        return pick(choice ? new byte[] { 8 }[0] : mutable);
    }

    public static void main(String[] args) {
        for (int branch = 0; branch < 2; branch++) {
            System.out.println(literal(branch != 0));
            System.out.println(finalFields(branch != 0));
            System.out.println(concatenation(branch != 0));
            System.out.println(noOverload(branch != 0));
            System.out.println(interfaceField(branch != 0));
            System.out.println(mutableField(branch != 0));
        }
    }
}

interface RawConstantChoice {
    String constant = "x";
    int value = ConstantChoiceContexts.a == ConstantChoiceContexts.b ? 0 : 1;
}
