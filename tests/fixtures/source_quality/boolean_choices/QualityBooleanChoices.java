public class QualityBooleanChoices {
    static int trace;
    static volatile boolean flag;

    static boolean probe(int value) {
        trace = trace * 31 + value;
        if (value == 9) throw new IllegalArgumentException();
        return value > 0;
    }

    static boolean eagerFailure(int value) { return false & probe(value); }
    static boolean eagerSuccess(int value) { return true | probe(value); }
    static boolean canonicalMask(int value) { return true & probe(value); }
    static boolean predicateMask(int value) { return true & (value & 2) == 0; }
    static boolean eagerSeed(int value) { return false | probe(value) | probe(value + 1); }
    static boolean normalizedEquality(boolean flag) { return (flag ? 1 : 0) == 1; }
    static boolean canonicalEquality(int value) { return probe(value) == true; }
    static boolean fieldPredicate(boolean selected) { return selected || (flag ? true : false); }
    boolean virtualProbe(int value) { return probe(value); }
    static boolean virtualPredicates(QualityBooleanChoices receiver, int first, int second) {
        return receiver.virtualProbe(first) && receiver.virtualProbe(second);
    }
    static String selectedText(boolean selected, String first, String second) {
        return !selected ? first : second;
    }
    static String opponent(String player, String[] players) {
        return player != players[0] ? players[0] : players[1];
    }
    static String nestedText(boolean selected, boolean enabled, String first, String second) {
        return !selected ? "default" : !enabled ? first.concat("!") : second.concat("?");
    }
    static boolean guardedEquals(Object first, Object second) {
        if (first == null && second == null) return true;
        if (first != null) return first.equals(second);
        return false;
    }

    static boolean toggle(int selector, int saved) {
        boolean selected = selector > 0;
        boolean enabled = saved > 0;
        return !selected && !enabled || true & selected & enabled;
    }

    static boolean alternateToggle(int selector, int saved) {
        boolean selected = selector > 0;
        boolean enabled = saved > 0;
        return selected && !enabled || true & !selected & enabled;
    }

    static boolean eagerTail(boolean selected, int value) {
        return !selected && value == 0 || true & selected & probe(value);
    }

    static boolean changingGuard(boolean selected, int value) {
        return !selected && (selected = probe(value)) || true & selected & value > 0;
    }

    static boolean volatileTail(boolean selected, int value) {
        return !selected && value == 0 || true & selected & flag;
    }

    static boolean selectedCalls(boolean selected, int first, int second) {
        return selected && probe(first) || !selected && probe(second);
    }

    static boolean readiness(int first, int second, int third) {
        boolean ready = true;
        if (first >= 0) ready &= probe(first);
        if (second >= 0) ready &= probe(second);
        if (third >= 0) ready &= probe(third);
        return ready;
    }

    static boolean maskedChoice(boolean selected, boolean raw) {
        return selected ? true & raw : false;
    }

    static int missingMask(int value) {
        if ((value & 1) == 0 || (value & 2) == 0) {
            trace += 7;
            return trace;
        }
        return 0;
    }

    public static void main(String[] args) {
        for (int first = -1; first < 3; first++) {
            for (int second = -1; second < 3; second++) {
                boolean selected = first > 0;
                flag = second > 0;
                trace = 0;
                System.out.println(eagerFailure(first) + ":" + eagerSuccess(first) + ":" + trace);
                trace = 0;
                System.out.println(toggle(first, second) + ":" + alternateToggle(first, second)
                    + ":" + eagerTail(selected, second) + ":" + changingGuard(selected, second)
                    + ":" + volatileTail(selected, second) + ":" + readiness(first, second, 1)
                    + ":" + maskedChoice(selected, flag) + ":" + selectedCalls(selected, first, second) + ":" + trace);
                trace = 0;
                System.out.println(canonicalMask(first) + ":" + normalizedEquality(selected)
                    + ":" + canonicalEquality(first) + ":" + fieldPredicate(selected)
                    + ":" + virtualPredicates(new QualityBooleanChoices(), first, second)
                    + ":" + predicateMask(first) + ":" + eagerSeed(second)
                    + ":" + selectedText(selected, "first", "second")
                    + ":" + opponent("first", new String[] { "first", "second" })
                    + ":" + nestedText(selected, flag, "first", "second")
                    + ":" + guardedEquals(selected ? "first" : null, flag ? "first" : null) + ":" + trace);
                System.out.println(missingMask(first) + ":" + trace);
            }
        }
        try { eagerFailure(9); } catch (IllegalArgumentException expected) { System.out.println("eager false"); }
        try { eagerSuccess(9); } catch (IllegalArgumentException expected) { System.out.println("eager true"); }
    }
}
