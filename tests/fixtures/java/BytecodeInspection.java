public class BytecodeInspection {
    static int field = 8;
    static int calls;

    static int zero() {
        calls++;
        return 0;
    }

    static int decision() {
        if (zero() != 0) return 30;
        return 40;
    }

    static int bindings(int x) {
        int next = x + 1;
        int doubled = next * 2;
        int unused = doubled + 9;
        return doubled;
    }

    static int reads() {
        int result = field;
        int unused = field;
        return result;
    }

    static int nullLength() {
        long[] values = null;
        try {
            return values.length;
        } catch (NullPointerException expected) {
            return 9;
        }
    }

    static int nullThrow() throws Exception {
        Exception failure = null;
        try {
            throw failure;
        } catch (NullPointerException expected) {
            return 12;
        }
    }

    public static void main(String[] args) throws Exception {
        System.out.println(decision());
        System.out.println(bindings(10));
        System.out.println(reads());
        System.out.println(nullLength());
        System.out.println(nullThrow());
        System.out.println(calls);
    }
}
