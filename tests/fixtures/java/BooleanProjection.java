// The test patches int arguments/results and storage descriptors to boolean.
// The bodies deliberately keep raw slots distinct from their nonzero predicates.
public class BooleanProjection {
    public static int raw;
    public static String trace = "";
    public BooleanProjection(int raw, int mode) {
        this(mode > 0 && callee(raw, mode) != 0, mode > 0 || callee(raw, mode) != 0);
    }
    protected BooleanProjection(boolean first, boolean second) {
        trace += first ? "t" : "f";
        trace += second ? "t" : "f";
    }
    public static int parameter(int raw, int mode) {
        if (mode == 1) return 0;
        if (mode == 2) return 1;
        return raw;
    }
    public static int opposite(int raw, int mode) {
        if (mode == 1) return 1;
        if (mode == 2) return 0;
        return raw;
    }
    public static int normalize(int raw, int mode) {
        if (raw != 0) return 1;
        return 0;
    }
    public static int normalizeChoice(int raw, int mode) {
        return raw != 0 ? 1 : 0;
    }
    public static int normalizeSelected(int raw, int mode) {
        return (mode < 0 ? raw : BooleanProjection.raw) != 0 ? 1 : 0;
    }
    public static int rawAlternative(int raw, int mode) {
        if (mode > 0 && raw != 0) return 1;
        if (mode > 0) return 0;
        return raw;
    }
    public static int canonical(int raw, int mode) {
        if (mode == 1) return 0;
        if (raw != 0) return 1;
        return 0;
    }
    public static int local(int raw, int mode) {
        int copy = raw;
        if (mode == 2) copy = 1;
        if (mode == 1) return 0;
        return copy;
    }
    public static int embedded(int raw, int mode) {
        int copy = 1;
        if ((copy = raw) != 0) trace += "a";
        if (mode == 1) return 0;
        return copy;
    }
    public static int loop(int raw, int mode) {
        int copy = 0;
        for (int i = 0; i < 3; i++) if (i == mode) copy = raw;
        if (mode == 1) return 1;
        return copy;
    }
    public static int canonicalLoop(int raw, int mode) {
        int copy = 0;
        for (int i = 0; i < 3; i++) if (i == mode) copy = 1;
        if (mode == -1) return 1;
        return copy;
    }
    public static int nested(int raw, int mode) {
        if (mode < 0) {
            if (mode == -1) return 1;
            return raw;
        }
        if (mode > 2) return 0;
        return raw;
    }
    public static int field(int mode) {
        if (mode == 1) return 0;
        if (mode == 2) return 1;
        return raw;
    }
    public static int normalizeField(int mode) {
        if (raw != 0) return 1;
        return 0;
    }
    public static int array(byte[] raw, int mode) {
        if (mode == 1) return 0;
        if (mode == 2) return 1;
        return raw[0];
    }
    public static int normalizeArray(byte[] raw, int mode) {
        if (raw[0] != 0) return 1;
        return 0;
    }
    public static int callee(int raw, int mode) {
        trace += "c";
        return raw;
    }
    public static int logicalAndProjection(int raw, int mode) {
        int result = 0;
        try { result = mode > 0 && callee(raw, mode) != 0 ? 1 : 0; }
        catch (IllegalArgumentException failed) { trace += "e"; }
        return result;
    }
    public static int logicalOrProjection(int raw, int mode) {
        int result = 0;
        try { result = mode > 0 || callee(raw, mode) != 0 ? 1 : 0; }
        catch (IllegalArgumentException failed) { trace += "e"; }
        return result;
    }
    public static int canonicalOther(int mode) { return mode == 2 ? 1 : 0; }
    public static int precedence(int raw, int mode) {
        return ((callee(raw, mode) | canonicalOther(mode)) != 0 ? 1 : 0)
            & (canonicalOther(mode) != 0 ? 1 : 0);
    }
    public static int calls(int raw, int mode) {
        if (mode == 1) return 0;
        return callee(raw, mode);
    }
    public static String eagerParameter(int raw, int mode) {
        return (raw & (mode == 1 ? 1 : 0)) != 0 ? "hit" : "miss";
    }
    public static String eagerField(int mode) {
        return (raw & (mode == 1 ? 1 : 0)) != 0 ? "hit" : "miss";
    }
    public static String eagerArray(byte[] raw, int mode) {
        return (raw[0] & callee(1, mode)) != 0 ? "hit" : "miss";
    }
    public static String eagerXor(int raw, int mode) {
        return (raw ^ (mode == 1 ? 1 : 0)) != 0 ? "hit" : "miss";
    }
    public static String eagerLoop(int raw, int mode) {
        int flags = raw;
        for (int i = 0; i < 3; i++) flags |= mode == i ? 1 : 0;
        return (flags & (mode < 0 ? 1 : 0)) != 0 ? "hit" : "miss";
    }
    public static String numericSink(int raw, int mode) {
        int flags = (mode > 0 ? 1 : 0) & (mode > 1 ? 1 : 0);
        if (flags != 0) trace += "n";
        return "number=" + (flags + 4);
    }
    public static int toggle(int selector, int mode) {
        int saved = raw;
        return selector == 0 && saved == 0 || (1 & selector & saved) != 0 ? 1 : 0;
    }
    public static int alternateToggle(int selector, int mode) {
        int saved = raw;
        return selector != 0 && saved == 0 || (1 & (selector == 0 ? 1 : 0) & saved) != 0 ? 1 : 0;
    }
    public static String predicateCapture(int raw, int mode) {
        int copy = raw != 0 ? 1 : 0;
        trace += "p";
        if (mode == 1) copy = 1;
        return copy != 0 ? "hit" : "miss";
    }
    public static String predicateArrayCapture(byte[] raw, int mode) {
        int copy = raw[0] != 0 ? 1 : 0;
        trace += "a";
        if (mode == 1) copy = 1;
        return copy != 0 ? "hit" : "miss";
    }
    public static String predicateCopyCapture(int raw, int mode) {
        int first = raw != 0 ? 1 : 0;
        int second = first;
        trace += "c";
        if (mode == 1) first = 1;
        if (first != 0) trace += "f";
        return second != 0 ? "hit" : "miss";
    }
    public static String predicateCatchCapture(int raw, int mode) {
        int copy = raw != 0 ? 1 : 0;
        try {
            trace += "t";
            if (mode == 1) throw new IllegalArgumentException();
            copy = 1;
        } catch (IllegalArgumentException caught) {
            return copy != 0 ? "caught-hit" : "caught-miss";
        }
        return copy != 0 ? "hit" : "miss";
    }
    public static int escapingCapture(int raw, int mode) {
        int copy = raw != 0 ? 1 : 0;
        trace += "e";
        if (mode == 1) copy = 1;
        return copy;
    }
    public static String exactOneCapture(int raw, int mode) {
        int copy = raw != 0 ? 1 : 0;
        trace += "x";
        if (mode == 1) copy = 1;
        return copy == 1 ? "hit" : "miss";
    }
    public static String maskCapture(int raw, int mode) {
        int copy = raw != 0 ? 1 : 0;
        trace += "m";
        if (mode == 1) copy = 1;
        return (copy & raw) != 0 ? "hit" : "miss";
    }
    public static String catchExactOneCapture(int raw, int mode) {
        int copy = raw != 0 ? 1 : 0;
        try {
            trace += "t";
            if (mode == 1) throw new IllegalArgumentException();
            copy = 1;
        } catch (IllegalArgumentException caught) {
            return copy == 1 ? "caught-hit" : "caught-miss";
        }
        return copy != 0 ? "hit" : "miss";
    }
    public static int predicateResultCapture(int raw, int mode) {
        int copy = raw != 0 ? 1 : 0;
        trace += "r";
        if (mode == 1) copy = 1;
        return copy != 0 ? 1 : 0;
    }
    public static int exactOneResultCapture(int raw, int mode) {
        int copy = raw != 0 ? 1 : 0;
        trace += "r";
        if (mode == 1) copy = 1;
        return copy == 1 ? 1 : 0;
    }
    public static int compoundAnd(int value, int mode) {
        if (mode > 0) value &= raw;
        return value;
    }
    public static int compoundOr(int value, int mode) {
        if (mode > 0) value |= raw;
        return value;
    }
    public static int compoundXor(int value, int mode) {
        if (mode > 0) value ^= raw;
        return value;
    }
}

class BooleanProjectionChild extends BooleanProjection {
    BooleanProjectionChild(int raw, int mode) {
        super(mode > 0 && callee(raw, mode) != 0, mode > 0 || callee(raw, mode) != 0);
    }
}
