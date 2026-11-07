public class ConstantReturns {
    static int trace;
    static final RuntimeException failure = new RuntimeException("same");

    static boolean alwaysFalse(boolean fail) {
        trace = trace * 31 + 1;
        if (fail) throw failure;
        return false;
    }

    static int seven(boolean fail) {
        trace = trace * 31 + 2;
        if (fail) throw failure;
        return 7;
    }

    static int varying(int mode) {
        trace = trace * 31 + 3;
        if (mode == 0) return 4;
        return 5;
    }

    static int branch(boolean fail) {
        if (alwaysFalse(fail)) return 1001;
        return trace;
    }

    static int select(boolean fail) {
        switch (seven(fail)) {
        case 7: return trace;
        default: return 1002;
        }
    }

    static int ordered(boolean first, boolean second) {
        if (alwaysFalse(first) == alwaysFalse(second)) return trace;
        return 1003;
    }

    static int initialize() {
        if (ConstantReturnHelper.result()) return 1004;
        return trace;
    }

    static int virtual(ConstantReturnBase receiver) {
        if (receiver.result()) return 7;
        return 9;
    }

    public static void main(String[] args) {
        for (int mode = 0; mode < 4; mode++) {
            for (int method = 0; method < 6; method++) {
                trace = 0;
                try {
                    int value;
                    if (method == 0) value = branch((mode & 1) != 0);
                    else if (method == 1) value = select((mode & 1) != 0);
                    else if (method == 2) value = ordered((mode & 1) != 0, (mode & 2) != 0);
                    else if (method == 3) value = initialize();
                    else if (method == 4) value = varying(mode) == 4 ? 17 : 19;
                    else value = virtual(mode == 0 ? new ConstantReturnBase() : new ConstantReturnChild());
                    System.out.println(new StringBuffer().append(value).append(':').append(trace).toString());
                } catch (Throwable caught) {
                    if (caught != failure) throw new Error("exception identity");
                    System.out.println(new StringBuffer().append("failure:").append(trace).toString());
                }
            }
        }
    }
}

class ConstantReturnHelper {
    static { ConstantReturns.trace = ConstantReturns.trace * 31 + 80; }
    static boolean result() {
        ConstantReturns.trace = ConstantReturns.trace * 31 + 4;
        return false;
    }
}
class ConstantReturnBase { boolean result() { return false; } }
class ConstantReturnChild extends ConstantReturnBase { boolean result() { return true; } }
