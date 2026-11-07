// The test patches int arguments/results and storage descriptors to boolean.
// The bodies deliberately keep raw slots distinct from their nonzero predicates.
public class BooleanProjection {
    public static int raw;
    public static String trace = "";
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
}
