public class CompoundTargets {
    static long trace;
    static CompoundTargets selected;
    byte[] bytes = { 127, -128, 43 };
    int[] numbers = { 2147483647, 12, -5 };
    volatile int count = 19;
    volatile int[] published = { 11 };

    static int mark(int value) { trace = trace * 31 + value; return value; }
    static CompoundTargets receiver() { mark(7); return selected; }
    static int index() { mark(3); return 0; }
    static int rhs() { mark(9); return 54; }

    void direct(int offset) { bytes[offset] ^= 54; }
    void computed(int offset) { bytes[offset * 2] ^= 54; }
    void fromArray(int offset, byte[] input) { bytes[offset] ^= input[offset + 1]; }
    static void receiverTarget() { receiver().bytes[index()] ^= rhs(); }
    static void fieldTarget() { receiver().count *= rhs(); }
    void publishedTarget() { published[index()] -= rhs(); }

    int replace() { mark(5); bytes = new byte[] { 2, 3, 4 }; return 54; }
    void replacedTarget() { bytes[index()] ^= replace(); }
    int replaceCount() { mark(6); count = 1000; return 7; }
    void replacedField() { count *= replaceCount(); }

    void sharedTarget() {
        byte[] captured = bytes;
        captured[index()] ^= replace();
        mark(captured[0]);
    }

    int savedOldValue() {
        int before = count;
        count = before * replaceCount();
        return before;
    }

    void sharedRight() {
        int saved;
        numbers[0] += (saved = rhs());
        mark(saved);
    }

    void narrowedOldValue() { count = (short) count + rhs(); }

    void finalTarget(boolean fail) {
        try {
            if (fail) throw new IllegalArgumentException();
            mark(1);
        } finally {
            bytes[index()] ^= rhs();
        }
    }

    int handlerSnapshot(int offset) {
        int observed = 2;
        try {
            bytes[offset] ^= mark(observed = 4);
            return observed;
        } catch (RuntimeException failure) {
            return observed + 20;
        }
    }

    void numeric(int shift, double fraction) {
        bytes[0] += fraction;
        bytes[1] *= 257;
        numbers[0] += 31;
        numbers[1] >>>= shift;
        count /= 3;
    }

    static void checked(CompoundTargets value, int offset) {
        try {
            value.bytes[offset] ^= rhs();
            mark(1);
        } catch (NullPointerException failure) {
            mark(21);
        } catch (ArrayIndexOutOfBoundsException failure) {
            mark(22);
        }
    }

    public static void main(String[] args) {
        CompoundTargets value = new CompoundTargets();
        selected = value;
        value.direct(0);
        value.computed(1);
        value.fromArray(1, new byte[] { 5, 6, 7 });
        receiverTarget();
        fieldTarget();
        value.publishedTarget();
        byte[] previous = value.bytes;
        value.replacedTarget();
        System.out.println(previous[0] + ":" + value.bytes[0]);
        value.replacedField();
        value.sharedTarget();
        System.out.println(value.count + ":" + value.published[0]);
        System.out.println(value.savedOldValue() + ":" + value.count);
        value.sharedRight();
        value.count = 65537;
        value.narrowedOldValue();
        System.out.println(value.count);
        value.finalTarget(false);
        try { value.finalTarget(true); } catch (IllegalArgumentException failure) { mark(8); }
        System.out.println(value.handlerSnapshot(0) + ":" + value.handlerSnapshot(5));
        value.numeric(35, 260.75);
        System.out.println(value.bytes[0] + ":" + value.bytes[1] + ":" + value.numbers[0] + ":" + value.numbers[1] + ":" + value.count);
        checked(null, 0);
        checked(value, -1);
        value.bytes = null;
        checked(value, 0);
        System.out.println(trace);
    }
}
