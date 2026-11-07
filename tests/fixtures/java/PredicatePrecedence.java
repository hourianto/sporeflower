public class PredicatePrecedence {
    static int trace;
    static int fail;
    static int kind;
    static int[] states = new int[2];
    static boolean[] flags = new boolean[3];

    static boolean step(boolean value, int marker) {
        trace = trace * 31 + marker;
        if (fail == marker) throw new IllegalStateException();
        return value;
    }

    static boolean notAndAnd(boolean a, boolean b, boolean c) {
        boolean selected = a && b;
        if (!selected && c) return true;
        return false;
    }
    static boolean notAndOr(boolean a, boolean b, boolean c) {
        boolean selected = a && b;
        if (!selected || c) return true;
        return false;
    }
    static boolean notOrAnd(boolean a, boolean b, boolean c) {
        boolean selected = a || b;
        if (!selected && c) return true;
        return false;
    }
    static boolean notOrOr(boolean a, boolean b, boolean c) {
        boolean selected = a || b;
        if (!selected || c) return true;
        return false;
    }
    static boolean rightNotAndAnd(boolean a, boolean b, boolean c) {
        boolean selected = a && b;
        if (c && !selected) return true;
        return false;
    }
    static boolean rightNotAndOr(boolean a, boolean b, boolean c) {
        boolean selected = a && b;
        if (c || !selected) return true;
        return false;
    }
    static boolean rightNotOrAnd(boolean a, boolean b, boolean c) {
        boolean selected = a || b;
        if (c && !selected) return true;
        return false;
    }
    static boolean rightNotOrOr(boolean a, boolean b, boolean c) {
        boolean selected = a || b;
        if (c || !selected) return true;
        return false;
    }
    static boolean effectNotAndAnd(boolean a, boolean b, boolean c) {
        boolean selected = step(a, 1) && step(b, 2);
        if (!selected && step(c, 3)) return true;
        return false;
    }
    static boolean effectNotAndOr(boolean a, boolean b, boolean c) {
        boolean selected = step(a, 1) && step(b, 2);
        if (!selected || step(c, 3)) return true;
        return false;
    }
    static boolean effectNotOrAnd(boolean a, boolean b, boolean c) {
        boolean selected = step(a, 1) || step(b, 2);
        if (!selected && step(c, 3)) return true;
        return false;
    }
    static boolean effectNotOrOr(boolean a, boolean b, boolean c) {
        boolean selected = step(a, 1) || step(b, 2);
        if (!selected || step(c, 3)) return true;
        return false;
    }
    static boolean effectRightNotAndAnd(boolean a, boolean b, boolean c) {
        boolean selected = step(a, 1) && step(b, 2);
        if (step(c, 3) && !selected) return true;
        return false;
    }
    static boolean effectRightNotAndOr(boolean a, boolean b, boolean c) {
        boolean selected = step(a, 1) && step(b, 2);
        if (step(c, 3) || !selected) return true;
        return false;
    }
    static boolean effectRightNotOrAnd(boolean a, boolean b, boolean c) {
        boolean selected = step(a, 1) || step(b, 2);
        if (step(c, 3) && !selected) return true;
        return false;
    }
    static boolean effectRightNotOrOr(boolean a, boolean b, boolean c) {
        boolean selected = step(a, 1) || step(b, 2);
        if (step(c, 3) || !selected) return true;
        return false;
    }
    static boolean nested(boolean a, boolean b, boolean c, boolean d) {
        boolean first = step(a, 1) && step(b, 2);
        boolean second = first || step(c, 3);
        if (!second && step(d, 4)) return true;
        return false;
    }
    static boolean mask(int kind, int state, int enabled) {
        boolean selected = kind == 12 && state == 2;
        if (!selected && (1 << kind & enabled) != 0) return true;
        return false;
    }
    static boolean maskFields() {
        boolean selected = kind == 12 && states[1] == 2;
        if (!selected && (1 << kind & 21238600) != 0) return true;
        return false;
    }
    static boolean fieldsAndAnd() {
        boolean selected = flags[0] && flags[1];
        if (!selected && flags[2]) return true;
        return false;
    }
    static boolean fieldsOrAnd() {
        boolean selected = flags[0] || flags[1];
        if (!selected && flags[2]) return true;
        return false;
    }
    static boolean positiveOrUnderAnd(boolean a, boolean b, boolean c) {
        if (a && (b || c ? true : false)) return true;
        return false;
    }
    static boolean positiveAndUnderOr(boolean a, boolean b, boolean c) {
        if (a || (b && c ? true : false)) return true;
        return false;
    }
    static boolean effectPositiveOrUnderAnd(boolean a, boolean b, boolean c) {
        if (step(a, 1) && (step(b, 2) || step(c, 3) ? true : false)) return true;
        return false;
    }
    static boolean effectPositiveAndUnderOr(boolean a, boolean b, boolean c) {
        if (step(a, 1) || (step(b, 2) && step(c, 3) ? true : false)) return true;
        return false;
    }
    static boolean eagerOr(boolean a, boolean b, boolean c) {
        boolean selected = step(a, 1) | step(b, 2);
        if (!selected && step(c, 3)) return true;
        return false;
    }
    static boolean eagerAnd(boolean a, boolean b, boolean c) {
        boolean selected = step(a, 1) & step(b, 2);
        if (!selected || step(c, 3)) return true;
        return false;
    }
}
