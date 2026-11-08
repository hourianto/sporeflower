public class QualityStorageDriver {
    static void reset(int failure) {
        QualityStorageExpressions.trace = 0;
        QualityStorageExpressions.failure = failure;
        QualityStorageExpressions.shared.count = 23;
        QualityStorageExpressions.currentArray = new int[] { 29, 31 };
        QualityStorageExpressions.currentLock = QualityStorageExpressions.firstLock;
    }

    static void printState() {
        System.out.println(QualityStorageExpressions.trace + ":"
            + QualityStorageExpressions.shared.count + ":"
            + QualityStorageExpressions.currentArray[0]);
    }

    static void effects(boolean selected, int failure) {
        reset(failure);
        try { QualityStorageExpressions.earlierResultBeforeReceiver(selected); }
        catch (IllegalArgumentException expected) { System.out.println("receiver:" + expected.getMessage()); }
        printState();
        reset(failure);
        try { QualityStorageExpressions.earlierResultBeforeArray(selected); }
        catch (IllegalArgumentException expected) { System.out.println("array:" + expected.getMessage()); }
        printState();
        reset(failure);
        try { System.out.println(QualityStorageExpressions.earlierResultBeforeArgument(selected)); }
        catch (IllegalArgumentException expected) { System.out.println("argument:" + expected.getMessage()); }
        printState();
        reset(failure);
        try { System.out.println(QualityStorageExpressions.precedingArgument(selected)); }
        catch (IllegalArgumentException expected) { System.out.println("preceding:" + expected.getMessage()); }
        printState();
        reset(failure);
        try { System.out.println(QualityStorageExpressions.localDependencyMutation(3)); }
        catch (IllegalArgumentException expected) { System.out.println("mutation:" + expected.getMessage()); }
        printState();
        reset(failure);
        try { System.out.println(QualityStorageExpressions.sharedExpressionResult(3)); }
        catch (IllegalArgumentException expected) { System.out.println("shared:" + expected.getMessage()); }
        printState();
        reset(failure);
        try { System.out.println(QualityStorageExpressions.conditionalOnlyUse(selected, 3)); }
        catch (IllegalArgumentException expected) { System.out.println("conditional:" + expected.getMessage()); }
        printState();
        reset(failure);
        try { System.out.println(QualityStorageExpressions.catchOnlyUse(3)); }
        catch (IllegalArgumentException expected) { System.out.println("catch:" + expected.getMessage()); }
        printState();
    }

    static void checkHandlerValue(String name, int actual, int expected) {
        if (actual != expected) {
            throw new IllegalArgumentException(name + ": expected " + expected + ", got " + actual);
        }
        System.out.println(name + ":" + actual);
    }

    static void handlerContinuations(int failure) {
        int assigned = failure == 1 ? 0 : (failure == 2 || failure == 3) ? 1 : 3;
        reset(failure);
        checkHandlerValue("continuation", QualityStorageExpressions.handlerContinuation(), assigned);
        printState();
        reset(failure);
        checkHandlerValue("pureContinuation", QualityStorageExpressions.pureHandlerContinuation(6),
            failure == 1 ? 0 : (failure == 2 || failure == 3) ? 7 : 5);
        printState();
        reset(failure);
        checkHandlerValue("break", QualityStorageExpressions.handlerBreak(), failure == -1 ? 5 : assigned);
        printState();
        reset(failure);
        checkHandlerValue("continue", QualityStorageExpressions.handlerContinue(), failure == -1 ? 5 : assigned);
        printState();
        reset(failure);
        checkHandlerValue("cleanupReturn", QualityStorageExpressions.handlerCleanup(), 5);
        checkHandlerValue("cleanupObserved", QualityStorageExpressions.shared.count, assigned);
        printState();
        reset(failure);
        QualityStorageExpressions.immediateHandlerArgument();
        printState();
        reset(failure);
        QualityStorageExpressions.pureHandlerArgument(6);
        printState();
    }

    public static void main(String[] args) {
        for (int mode = 0; mode < 4; mode++) {
            boolean selected = (mode & 1) != 0;
            reset(99);
            QualityStorageExpressions.shared.enabled = (mode & 2) != 0;
            QualityStorageExpressions.shared.fieldChoice(selected);
            System.out.println(QualityStorageExpressions.shared.enabled);
            QualityStorageExpressions.arrayChoice(QualityStorageExpressions.currentArray, 0, selected);
            QualityStorageExpressions.argumentChoice(selected);
            QualityStorageExpressions.nestedArrayChoice(QualityStorageExpressions.currentArray, 1, 31);
            for (int value = -2; value <= 2; value++) {
                System.out.println(QualityStorageExpressions.branchReturn(selected, value));
            }
            System.out.println(QualityStorageExpressions.narrowChoice(selected, 257, -129));
            System.out.println(QualityStorageExpressions.referenceChoice(selected, "value"));
            System.out.println(QualityStorageExpressions.nullableString(selected));
            System.out.println(QualityStorageExpressions.mirroredNullableString(selected));
            System.out.println(QualityStorageExpressions.retainedChoiceAfterEffect(selected));
            printState();
            for (int failure = -1; failure <= 9; failure++) effects(selected, failure);
        }

        handlerContinuations(-1);
        for (int failure = 1; failure <= 4; failure++) handlerContinuations(failure);

        int[] values = { Integer.MIN_VALUE, -1, 0, 1, Integer.MAX_VALUE };
        for (int i = 0; i < values.length; i++) {
            reset(99);
            int value = values[i];
            System.out.println(QualityStorageExpressions.integerIdentity(value) + ":"
                + QualityStorageExpressions.integerCancellation(value) + ":"
                + QualityStorageExpressions.widenedCopy(value) + ":"
                + QualityStorageExpressions.longIdentity(value) + ":"
                + QualityStorageExpressions.widenedByteCopy((byte) value) + ":"
                + QualityStorageExpressions.widenedIdentity((byte) value));
            System.out.println(QualityStorageExpressions.oldFieldSnapshot(value));
            System.out.println(QualityStorageExpressions.oldArraySnapshot(value));
            printState();
        }
        long[] wide = { Long.MIN_VALUE, -1L, 0L, 1L, Long.MAX_VALUE };
        for (int i = 0; i < wide.length; i++) {
            System.out.println(QualityStorageExpressions.longCancellation(wide[i]));
        }
        double[] floating = { -0.0D, 0.0D, 0.5D, 9007199254740992.0D,
            Double.NaN, Double.POSITIVE_INFINITY, Double.NEGATIVE_INFINITY };
        for (int i = 0; i < floating.length; i++) {
            System.out.println(Double.doubleToLongBits(QualityStorageExpressions.floatingIdentity(floating[i]))
                + ":" + Double.doubleToLongBits(QualityStorageExpressions.floatingCancellation(floating[i])));
        }
        System.out.println(QualityStorageExpressions.stringIdentity("value"));
        System.out.println(QualityStorageExpressions.stringIdentity(null));

        for (int failure = -1; failure <= 4; failure++) {
            reset(failure);
            try { System.out.println(QualityStorageExpressions.returnedBeforeFinally(2)); }
            catch (IllegalArgumentException expected) { System.out.println("cleanup:" + expected.getMessage()); }
            printState();
        }
        for (int selector = -1; selector <= 5; selector++) {
            reset(99);
            System.out.println(QualityStorageExpressions.sharedSwitchResult(selector));
            System.out.println(QualityStorageExpressions.terminalResultObserved(selector));
            printState();
        }
        reset(-1);
        System.out.println(QualityStorageExpressions.monitorCapture());
        System.out.println(QualityStorageExpressions.ownsLock(QualityStorageExpressions.firstLock) + ":"
            + QualityStorageExpressions.ownsLock(QualityStorageExpressions.secondLock));
        printState();
        reset(-1);
        System.out.println(QualityStorageExpressions.directMonitor());
        printState();
    }
}
