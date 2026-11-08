public class QualityBranchFacts {
    static int count;
    static int trace;
    static volatile int changing;
    int value;

    static int completedReceiver(String text) {
        int length = text.length();
        if (text != null) return length + 1;
        return -1;
    }

    static int completedArray(int[] values) {
        int length = values.length;
        if (values == null) return -1;
        return length;
    }

    static int conditionalReceiver(boolean selected, String text) {
        int length = selected ? text.length() : 0;
        if (text == null) return -1;
        return length;
    }

    static boolean overwrittenReceiver(String text) {
        boolean same = text.equals(text = null);
        return same || text == null;
    }

    static int caughtReceiver(String text) {
        try { text.length(); } catch (NullPointerException expected) { trace++; }
        return text == null ? -1 : 1;
    }

    static int clamp() {
        if (count > 0) {
            if (count <= 0) count = 0;
            else count--;
        }
        return count;
    }

    int instanceClamp() {
        if (value > 0) {
            if (value <= 0) value = 0;
            else value--;
        }
        return value;
    }

    int repeatedLoopBound() {
        int sum = 0;
        for (int index = 0; index < value && index < value; index++) sum += index;
        return sum;
    }

    static int repeatedVolatileLoopBound() {
        int sum = 0;
        for (int index = 0; index < changing && index < changing; index++) sum += index;
        return sum;
    }

    static int mutate() { trace++; return count = -3; }

    static int callChangesField() {
        if (count > 0) {
            mutate();
            if (count <= 0) return 7;
        }
        return 8;
    }

    static int writeChangesField() {
        if (count > 0) {
            count = -3;
            if (count <= 0) return 7;
        }
        return 8;
    }

    static int volatileReads() {
        if (changing > 0 && changing <= 0) return 7;
        return 8;
    }

    static int monitorBoundary(Object monitor) {
        if (count > 0) {
            synchronized (monitor) { if (count <= 0) return 7; }
        }
        return 8;
    }

    static int nonzeroMask() {
        if (count == 0) return 1;
        if (count == 2) return 2;
        if (0 != (count | 1)) return 3;
        return 4;
    }

    static int effectfulMask() { return (mutate() | 1) != 0 ? 3 : 4; }
    static int volatileMask() { return (changing | 1) != 0 ? 3 : 4; }

    static class Trigger {
        static int value = mutate();
    }

    static int initializerChangesField() {
        if (count > 0) {
            int saved = Trigger.value;
            if (count <= 0) return saved + 7;
        }
        return 8;
    }

    static class PairedTrigger {
        static int value = initialize();
        static int initialize() {
            count = 2;
            return 1;
        }
    }

    static int initializerBetweenComparedFields(int[] result) {
        if (count == PairedTrigger.value) {
            result[0] = 2;
            if (count == PairedTrigger.value) result[0] = 1;
        } else {
            result[0] = 3;
        }
        return result[0];
    }

    static boolean mutateBoolean() {
        count = -1;
        return true;
    }

    static int eagerMaskChangesField() {
        return count > 0 ? (mutateBoolean() & (count > 0) ? 1 : 2) : 3;
    }

    static int guardedComposite(boolean blocked, String text) {
        if (blocked) return 1;
        int length = text.length();
        if (length > 2 && count > 0 || blocked) return 2;
        return 3;
    }

    static int guardedConjunction(boolean enabled, int value) {
        if (!enabled) return 1;
        mutate();
        if (value > 0 && enabled) return 2;
        return 3;
    }

    static int compoundDereference(String text) {
        if (text != null && text.length() > 0 && text != null) return 1;
        return 2;
    }

    static int changedGuard(boolean blocked, int value) {
        if (blocked) return 1;
        blocked = mutateSelected(value);
        if (value > 0 || blocked) return 2;
        return 3;
    }

    static boolean mutateSelected(int value) { trace++; return value == 0; }

    static int effectfulPrefix(boolean blocked) {
        if (blocked) return 1;
        if (mutateBoolean() || blocked) return 2;
        return 3;
    }

    static int determiningSuffix(boolean enabled) {
        if (!enabled) return 1;
        if (mutateBoolean() || enabled) return 2;
        return 3;
    }

    static int switchDefault(int mode) {
        switch (mode) {
            case 1:
            case 2:
                trace += 2;
                break;
            default:
                if (mode == 1 || mode == 2) return 99;
                switch (mode) {
                    case 1:
                    case 2:
                        return 98;
                    default:
                        trace += 4;
                }
                break;
        }
        return trace;
    }

    static int switchSelectedCase(int mode) {
        switch (mode) {
            case 2:
                switch (mode) {
                    case 2: trace += 2; break;
                    default: return 98;
                }
                break;
            default:
                trace += 4;
        }
        return trace;
    }

    static int switchFallthrough(int mode) {
        switch (mode) {
            case 1:
                trace++;
            default:
                switch (mode) {
                    case 1: trace += 10; break;
                    default: trace += 20;
                }
        }
        return trace;
    }

    static int switchChangedSelector(int mode) {
        switch (mode) {
            case 1:
            case 2:
                trace += 2;
                break;
            default:
                mode = changing;
                switch (mode) {
                    case 1:
                    case 2:
                        trace += 4;
                        break;
                    default:
                        trace += 8;
                }
        }
        return trace;
    }

    static int switchEarlyBreak(int mode, int value) {
        if (mode != 2) return 0;
        switch (mode) {
            case 2:
                if (value < 0) break;
                trace += value;
                break;
            default:
                trace += 8;
        }
        return trace;
    }

    static int switchSelectedFallthrough(int mode) {
        if (mode != 2) return 0;
        switch (mode) {
            case 2: trace += 2;
            case 3: trace += 3; break;
            default: trace += 8;
        }
        return trace;
    }

    public static void main(String[] args) {
        System.out.println(completedReceiver("abc") + ":" + completedArray(new int[3]));
        try { completedReceiver(null); } catch (NullPointerException expected) { System.out.println("null receiver"); }
        try { completedArray(null); } catch (NullPointerException expected) { System.out.println("null array"); }
        System.out.println(conditionalReceiver(false, null) + ":" + conditionalReceiver(true, "a")
            + ":" + overwrittenReceiver("abc") + ":" + caughtReceiver(null) + ":" + caughtReceiver("a"));
        QualityBranchFacts instance = new QualityBranchFacts();
        for (int input = -2; input < 4; input++) {
            count = input;
            instance.value = input;
            changing = input;
            System.out.println(instance.repeatedLoopBound() + ":" + repeatedVolatileLoopBound());
            System.out.println(clamp() + ":" + instance.instanceClamp() + ":" + nonzeroMask()
                + ":" + volatileReads() + ":" + volatileMask() + ":" + monitorBoundary(instance));
            count = input;
            System.out.println(callChangesField() + ":" + trace);
            count = input;
            System.out.println(writeChangesField() + ":" + effectfulMask() + ":" + trace);
        }
        count = 1;
        System.out.println(initializerChangesField() + ":" + trace);
        count = 1;
        System.out.println(initializerBetweenComparedFields(new int[1]));
        System.out.println(initializerBetweenComparedFields(new int[1]));
        count = 1;
        System.out.println(eagerMaskChangesField());
        for (int input = -1; input < 2; input++) {
            for (int flag = 0; flag < 2; flag++) {
                count = input;
                boolean selected = flag != 0;
                System.out.println(guardedComposite(selected, "test") + ":"
                    + guardedConjunction(selected, input) + ":"
                    + changedGuard(selected, input) + ":" + effectfulPrefix(selected)
                    + ":" + determiningSuffix(selected) + ":" + trace);
            }
        }
        System.out.println(compoundDereference(null) + ":" + compoundDereference("")
            + ":" + compoundDereference("text"));
        for (int mode = 0; mode < 4; mode++) {
            changing = mode;
            trace = 0;
            System.out.println(switchDefault(mode) + ":" + switchSelectedCase(mode)
                + ":" + switchFallthrough(mode) + ":" + switchChangedSelector(mode)
                + ":" + switchEarlyBreak(mode, -1) + ":" + switchEarlyBreak(mode, 1)
                + ":" + switchSelectedFallthrough(mode));
        }
    }
}
