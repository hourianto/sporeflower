/** Authored examples of expression destinations and observable storage boundaries. */
public class QualityStorageExpressions {
    static int trace;
    static int failure;
    static final QualityStorageExpressions shared = new QualityStorageExpressions();
    static final Object firstLock = new Object();
    static final Object secondLock = new Object();
    static Object currentLock = firstLock;
    static int[] currentArray = new int[2];
    boolean enabled;
    int count;

    static int mark(int value) {
        trace = trace * 31 + value;
        if (failure == value) throw new IllegalArgumentException(String.valueOf(value));
        return value;
    }

    static boolean probe(int value) { return mark(value) > 0; }
    static int combine(int first, int second) { return first * 100 + second; }
    static void consume(int value) { mark(value); }
    static QualityStorageExpressions receiver() { mark(7); return shared; }
    static int[] array() { mark(8); return currentArray; }
    static int index() { mark(9); return 0; }
    static Object lock() { mark(10); return currentLock; }
    static boolean ownsLock(Object monitor) {
        try {
            monitor.notifyAll();
            return true;
        } catch (IllegalMonitorStateException expected) {
            return false;
        }
    }

    void fieldChoice(boolean selected) {
        boolean old = enabled;
        boolean replacement = selected ? old : !old;
        enabled = replacement;
        mark(old ? 1 : 2);
    }

    static void arrayChoice(int[] values, int offset, boolean selected) {
        int result;
        if (selected) result = 17;
        else result = -19;
        values[offset] = result;
    }

    static void argumentChoice(boolean selected) {
        int result;
        if (selected) result = 17;
        else result = 19;
        consume(result);
    }

    static int retainedChoiceAfterEffect(boolean selected) {
        mark(7);
        int result;
        if (selected) result = mark(1);
        else result = mark(2);
        return result + result;
    }

    static void nestedArrayChoice(int[] values, int offset, int color) {
        int base = values[offset] & 255;
        int alpha = 255;
        if (base == color) alpha = 0;
        values[offset] = base + (alpha << 8);
    }

    static boolean branchReturn(boolean selected, int value) {
        if (selected) {
            boolean result = value >= 0 && probe(value);
            return result;
        }
        return probe(-value);
    }

    static int sharedSwitchResult(int selector) {
        int result = -1;
        switch (selector) {
            case 0: result = mark(1); break;
            case 1: result = mark(2); break;
            case 2: result = mark(3); break;
            case 3: result = mark(4); break;
            case 4: result = mark(5); break;
            default: break;
        }
        return result;
    }

    static int terminalResultObserved(int selector) {
        int result = -1;
        try {
            switch (selector) {
                case 0: result = mark(1); return result;
                case 1: result = mark(2); return result;
                default: return result;
            }
        } finally {
            shared.count = result;
        }
    }

    static String widenedCopy(int value) {
        int result = mark(value);
        long widened = result;
        return kind(widened);
    }

    static int integerIdentity(int value) { return value + 0; }
    static int integerCancellation(int value) { return value - 1 + 1; }
    static long longCancellation(long value) { return value + 1L - 1L; }

    static void earlierResultBeforeReceiver(boolean selected) {
        int result = selected ? mark(3) : mark(4);
        receiver().count = result;
    }

    static void earlierResultBeforeArray(boolean selected) {
        int result = selected ? mark(3) : mark(4);
        array()[index()] = result;
    }

    static int earlierResultBeforeArgument(boolean selected) {
        int result = selected ? mark(3) : mark(4);
        return combine(mark(5), result);
    }

    static int oldFieldSnapshot(int replacement) {
        int old = shared.count;
        shared.count = mark(replacement);
        return combine(old, shared.count);
    }

    static int oldArraySnapshot(int replacement) {
        int old = currentArray[0];
        currentArray[0] = mark(replacement);
        return combine(old, currentArray[0]);
    }

    static int returnedBeforeFinally(int value) {
        int result = value;
        try {
            result = mark(value + 1);
            return result;
        } finally {
            result = mark(value + 2);
            shared.count = result;
        }
    }

    static int monitorCapture() {
        Object captured = lock();
        synchronized (captured) {
            currentLock = secondLock;
            mark(ownsLock(captured) ? 11 : 12);
            return ownsLock(currentLock) ? 13 : 14;
        }
    }

    static int directMonitor() {
        synchronized (lock()) {
            mark(11);
            return 12;
        }
    }

    static int precedingArgument(boolean selected) {
        return combine(mark(1), selected ? mark(2) : mark(3));
    }

    static int localDependencyMutation(int value) {
        int old = value + 1;
        value = mark(6);
        return combine(old, value);
    }

    static int sharedExpressionResult(int value) {
        int result = mark(value);
        return combine(result, result);
    }

    static int conditionalOnlyUse(boolean selected, int value) {
        int result = mark(value);
        return selected ? result : 0;
    }

    static int catchOnlyUse(int value) {
        int result = mark(value);
        try {
            consume(7);
            return 0;
        } catch (IllegalArgumentException expected) {
            return result;
        }
    }

    static void observeHandlerValue(int value) {
        shared.count = value;
        mark(2);
    }

    static int handlerContinuation() {
        int result = 0;
        try {
            result = mark(1);
            observeHandlerValue(result);
            result = mark(3);
            consume(4);
        } catch (RuntimeException expected) {
        }
        return result;
    }

    static int pureHandlerContinuation(int value) {
        int result = 0;
        try {
            consume(1);
            result = value + 1;
            consume(3);
            observeHandlerValue(result);
            result = 5;
        } catch (RuntimeException expected) {
        }
        return result;
    }

    static int handlerBreak() {
        int result = 0;
        for (int iteration = 0; iteration < 2; iteration++) {
            consume(6);
            try {
                result = mark(1);
                observeHandlerValue(result);
                result = mark(3);
                consume(4);
            } catch (RuntimeException expected) {
                break;
            }
            result = 5;
            consume(7);
        }
        consume(8);
        return result;
    }

    static int handlerContinue() {
        int result = 0;
        int observed = 0;
        for (int iteration = 0; iteration < 2; iteration++) {
            observed = observed * 10 + result;
            try {
                result = mark(1);
                observeHandlerValue(result);
                result = mark(3);
                consume(4);
            } catch (RuntimeException expected) {
                continue;
            }
            result = 5;
            consume(6);
        }
        return observed;
    }

    static int handlerCleanup() {
        int result = 0;
        try {
            result = mark(1);
            observeHandlerValue(result);
            result = mark(3);
            consume(4);
        } catch (RuntimeException expected) {
        } finally {
            shared.count = result;
            result = 5;
        }
        return result;
    }

    static void immediateHandlerArgument() {
        try {
            int result = mark(1);
            observeHandlerValue(result);
        } catch (RuntimeException expected) {
        }
    }

    static void pureHandlerArgument(int value) {
        try {
            int result = value + 1;
            consume(3);
            observeHandlerValue(result);
        } catch (RuntimeException expected) {
        }
    }

    static String kind(byte value) { return "byte:" + value; }
    static String kind(int value) { return "int:" + value; }
    static String kind(long value) { return "long:" + value; }
    static String kind(Object value) { return "object:" + value; }
    static String kind(String value) { return "string:" + value; }

    static String narrowChoice(boolean selected, int first, int second) {
        byte result = selected ? (byte) first : (byte) second;
        return kind(result);
    }

    static String widenedIdentity(byte value) { return kind(value + 0); }
    static String widenedByteCopy(byte value) {
        int widened = value;
        return kind(widened);
    }
    static String longIdentity(int value) { return kind(value + 0L); }

    static String referenceChoice(boolean selected, String text) {
        Object result = selected ? text : null;
        return kind(result);
    }

    static String nullableString(boolean selected) {
        String result = selected ? null : new String("value");
        return result;
    }

    static String mirroredNullableString(boolean selected) {
        String result = selected ? new String("value") : null;
        return result;
    }

    static double floatingIdentity(double value) { return value + 0.0D; }
    static double floatingCancellation(double value) { return value + 1.0D - 1.0D; }
    static String stringIdentity(String value) { return value + 0; }
}
