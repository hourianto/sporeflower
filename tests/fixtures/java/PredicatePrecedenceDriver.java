public class PredicatePrecedenceDriver {
    static boolean evaluate(int kind, boolean a, boolean b, boolean c) {
        switch (kind) {
            case 0: return PredicatePrecedence.notAndAnd(a, b, c);
            case 1: return PredicatePrecedence.notAndOr(a, b, c);
            case 2: return PredicatePrecedence.notOrAnd(a, b, c);
            case 3: return PredicatePrecedence.notOrOr(a, b, c);
            case 4: return PredicatePrecedence.rightNotAndAnd(a, b, c);
            case 5: return PredicatePrecedence.rightNotAndOr(a, b, c);
            case 6: return PredicatePrecedence.rightNotOrAnd(a, b, c);
            case 7: return PredicatePrecedence.rightNotOrOr(a, b, c);
            case 8: return PredicatePrecedence.effectNotAndAnd(a, b, c);
            case 9: return PredicatePrecedence.effectNotAndOr(a, b, c);
            case 10: return PredicatePrecedence.effectNotOrAnd(a, b, c);
            case 11: return PredicatePrecedence.effectNotOrOr(a, b, c);
            case 12: return PredicatePrecedence.effectRightNotAndAnd(a, b, c);
            case 13: return PredicatePrecedence.effectRightNotAndOr(a, b, c);
            case 14: return PredicatePrecedence.effectRightNotOrAnd(a, b, c);
            case 15: return PredicatePrecedence.effectRightNotOrOr(a, b, c);
            case 16: return PredicatePrecedence.positiveOrUnderAnd(a, b, c);
            case 17: return PredicatePrecedence.positiveAndUnderOr(a, b, c);
            case 18: return PredicatePrecedence.effectPositiveOrUnderAnd(a, b, c);
            case 19: return PredicatePrecedence.effectPositiveAndUnderOr(a, b, c);
            case 20: return PredicatePrecedence.eagerOr(a, b, c);
            default: return PredicatePrecedence.eagerAnd(a, b, c);
        }
    }
    public static void main(String[] args) {
        for (int kind = 0; kind < 22; kind++) {
            for (int bits = 0; bits < 8; bits++) {
                for (int fail = 0; fail < 4; fail++) {
                    PredicatePrecedence.trace = 0;
                    PredicatePrecedence.fail = fail;
                    String result;
                    try { result = "value=" + evaluate(kind, (bits & 1) != 0, (bits & 2) != 0, (bits & 4) != 0); }
                    catch (IllegalStateException expected) { result = "throw"; }
                    System.out.println(kind + ":" + bits + ":" + fail + ":" + result + ":" + PredicatePrecedence.trace);
                }
            }
        }
        for (int bits = 0; bits < 16; bits++) {
            for (int fail = 0; fail < 5; fail++) {
                PredicatePrecedence.trace = 0;
                PredicatePrecedence.fail = fail;
                String result;
                try { result = "value=" + PredicatePrecedence.nested((bits & 1) != 0, (bits & 2) != 0, (bits & 4) != 0, (bits & 8) != 0); }
                catch (IllegalStateException expected) { result = "throw"; }
                System.out.println("nested:" + bits + ":" + fail + ":" + result + ":" + PredicatePrecedence.trace);
            }
        }
        int[] masks = new int[] { 0, 21238600, -1 };
        for (int kind = 0; kind < 32; kind++) {
            for (int state = 0; state < 4; state++) {
                for (int mask = 0; mask < masks.length; mask++) {
                    System.out.println("mask:" + kind + ":" + state + ":" + mask + ":" + PredicatePrecedence.mask(kind, state, masks[mask]));
                }
                PredicatePrecedence.kind = kind;
                PredicatePrecedence.states[1] = state;
                System.out.println("fields:" + kind + ":" + state + ":" + PredicatePrecedence.maskFields());
            }
        }
        for (int bits = 0; bits < 8; bits++) {
            PredicatePrecedence.flags[0] = (bits & 1) != 0;
            PredicatePrecedence.flags[1] = (bits & 2) != 0;
            PredicatePrecedence.flags[2] = (bits & 4) != 0;
            System.out.println("fields-and:" + bits + ":" + PredicatePrecedence.fieldsAndAnd());
            System.out.println("fields-or:" + bits + ":" + PredicatePrecedence.fieldsOrAnd());
        }
    }
}
