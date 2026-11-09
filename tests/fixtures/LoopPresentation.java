public class LoopPresentation {
    static int trace;
    static final Object lock = new Object();

    static int effect(int value) {
        trace = trace * 7 + value;
        return value;
    }

    static boolean again(int value, int limit) {
        effect(20 + value);
        if (limit < 0) throw new IllegalArgumentException();
        return value < limit;
    }

    static int count(Object[] values) {
        int count = 0;
        for (int i = 0; i < values.length; i++) {
            if (values[i] == null) continue;
            count++;
        }
        return count;
    }

    static int flags(boolean[] values) {
        int count = 0;
        for (int i = 0; i < values.length; i++) {
            if (!values[i]) continue;
            count++;
        }
        return count;
    }

    static int filteredEffects(int limit) {
        int count = 0;
        for (int i = 0; i < limit; i = effect(i + 1)) {
            if (effect(i) % 2 == 0) continue;
            count += effect(i + 10);
        }
        return count;
    }

    static int largerGuard(Object[] values) {
        int count = 0;
        for (int i = 0; i < values.length; i++) {
            if (values[i] == null) continue;
            count++;
            effect(i);
            values[i] = new Object();
        }
        return count;
    }

    static int compoundGuard(int[] values) {
        int count = 0;
        for (int i = 0; i < values.length; i++) {
            if (values[i] < 0 || values[i] > 5) continue;
            count++;
        }
        return count;
    }

    static boolean matchingTail(int[] values, int wanted) {
        for (int i = 0; i < values.length; i = effect(i + 1)) {
            if (effect(values[i]) != wanted) continue;
            return true;
        }
        return false;
    }

    static int alternativeTail(int limit) {
        int total = 0;
        for (int i = 0; i < limit; i = effect(i + 1)) {
            if (effect(i) % 2 == 0) {
                total += effect(10);
                continue;
            }
            total += effect(20);
            effect(30);
        }
        return total;
    }

    static int floatingTail(double[] values) {
        int count = 0;
        for (int i = 0; i < values.length; i++) {
            if (!(values[i] < 1.0)) continue;
            count++;
            effect(i);
        }
        return count;
    }

    static int largeTail(int limit) {
        for (int i = 0; i < limit; i++) {
            if (effect(i) % 2 == 0) continue;
            effect(10);
            effect(20);
            effect(30);
            effect(40);
            effect(50);
        }
        return trace;
    }

    static int labelledTail(int limit) {
        outer: for (int i = 0; i < limit; i = effect(i + 1)) {
            for (int j = 0; j < limit; j++) {
                if (effect(j) == 1) continue outer;
                effect(i + 10);
            }
            effect(30);
        }
        return trace;
    }

    static int guardedCleanup(int limit) {
        int count = 0;
        for (int i = 0; i < limit; i = effect(i + 1)) {
            try {
                if (i % 2 == 0) {
                    effect(10);
                    continue;
                } else {
                    effect(11);
                }
                count++;
            } finally {
                effect(12);
            }
        }
        return count;
    }

    static int postTest(int limit) {
        int i = 0;
        do {
            effect(++i);
        } while (again(i, limit));
        return i;
    }

    static int nestedPostTest(int limit) {
        int i = 0;
        do {
            for (int j = 0; j < 3; j++) {
                if (j == 1) continue;
                effect(j);
                effect(j + 5);
            }
            i++;
        } while (again(i, limit));
        return i;
    }

    static int lazyPostTest(int limit) {
        int i = 0;
        do {
            effect(++i);
        } while (i < limit && again(i, limit));
        return i;
    }

    static int skipTest(int limit) {
        int i = 0;
        while (true) {
            i++;
            if (i < 3) continue;
            effect(i);
            if (again(i, limit)) continue;
            break;
        }
        return i;
    }

    static int scopedTest(int limit) {
        int i = 0;
        while (true) {
            int saved = effect(++i);
            if (saved < limit) continue;
            break;
        }
        return i;
    }

    static int retainedScopedTest(int limit) {
        int i = 0;
        while (true) {
            int saved = effect(++i);
            effect(saved + 10);
            if (saved < limit) continue;
            break;
        }
        return i;
    }

    static int labelledSkip(int limit) {
        int i = 0;
        outer: while (true) {
            i++;
            for (int j = 0; j < 2; j++) {
                if (i < 3) continue outer;
                effect(j);
            }
            if (again(i, limit)) continue;
            break;
        }
        return i;
    }

    static int protectedPostTest(int limit) {
        int i = 0;
        do {
            try {
                synchronized (lock) {
                    switch (i % 3) {
                        case 0: effect(2); break;
                        case 1: effect(3); break;
                        default: effect(4);
                    }
                }
            } finally {
                effect(9);
            }
            i++;
        } while (again(i, limit));
        return i;
    }

    static boolean whitespace(char value) {
        effect(value);
        return value == ' ' || value == '\t';
    }

    static int scanCursor;
    static char nextScan(char[] input) { return input[scanCursor++]; }

    static int readAfterPostTest(char[] input) {
        scanCursor = 0;
        char value;
        do {
            value = nextScan(input);
        } while (whitespace(value));
        if (value == '\000') return -1;
        return value + scanCursor;
    }

    static int countdown(int limit) {
        int total = 0;
        while (--limit >= 0) {
            if ((limit & 1) == 0) continue;
            total += effect(limit);
        }
        return total;
    }

    static int protectedCountdown(int limit, int[] values) {
        try {
            while (--limit >= 0) effect(values[limit]);
        } catch (RuntimeException e) {
            effect(limit);
        }
        return limit;
    }

    static int savedCountdown(int limit) {
        int total = 0;
        while (true) {
            int saved = limit;
            limit--;
            if (saved < 0) break;
            total += effect(saved);
        }
        return total;
    }

    static int wrappingCountdown(int limit) {
        int turns = 0;
        while (--limit >= 0) {
            effect(limit);
            if (++turns == 3) break;
        }
        return limit;
    }

    static int boundedIndex(int[] values, int wanted) {
        int index = 0;
        for (; true; index++) {
            if (index >= values.length) { index = -1; break; }
            if (values[index] != wanted) continue;
            break;
        }
        return index;
    }

    static boolean boundedFlag(int[] values, int wanted) {
        boolean found;
        for (int index = 0; true; index++) {
            if (index >= values.length) { found = false; break; }
            if (values[index] < 0) continue;
            if (effect(values[index]) != wanted) continue;
            found = true;
            break;
        }
        return found;
    }

    static int boundedCounterReturn(int[] values, int wanted) {
        for (int index = 0; true; index++) {
            if (index >= values.length) return index;
            if (values[index] == wanted) return -1;
        }
    }

    static int effectfulSearch(int[] values, int wanted) {
        int index;
        for (index = 0; true; index++) {
            if (index >= values.length) { index = -1; break; }
            if (effect(values[index]) == wanted) break;
        }
        return effect(index);
    }

    static int repeatedSearch(int[] values) {
        int sum = 0;
        for (int wanted = 0; wanted < 5; wanted++) {
            int index;
            for (index = 0; true; index++) {
                if (index >= values.length) { index = -1; break; }
                if (effect(values[index]) == wanted) break;
            }
            sum += effect(index);
        }
        return sum;
    }

    static int observedExhaustion(int[] values, int wanted) {
        int result = -7;
        try {
            int index = 0;
            while (true) {
                if (index >= values.length) { result = -1; break; }
                if (effect(values[index]) == wanted) { result = index; break; }
                index++;
            }
        } catch (RuntimeException e) { return effect(result); }
        return effect(result);
    }

    static boolean persistentMatch(int[][] values, int wanted) {
        boolean found = false;
        outer: for (int i = 0; i < values.length; i++) {
            for (int j = 0; j < values[i].length; j++) {
                if (effect(values[i][j]) != wanted) continue outer;
            }
            found = true;
        }
        return found;
    }

    static int cleanupObservedSearch(int[] values, int wanted, boolean fail) {
        int result = -3;
        try {
            try {
                result = boundedIndex(values, wanted);
                return result;
            } finally {
                effect(result);
                if (fail) throw new IllegalArgumentException();
                result = 9;
            }
        } catch (RuntimeException e) { return effect(result); }
    }

    public static void main(String[] args) {
        Object[] values = new Object[] {null, new Object(), null, new Object()};
        System.out.println(count(values) + ":" + flags(new boolean[] {false, true, true, false}));
        System.out.println(largerGuard(values) + ":" + compoundGuard(new int[] {-1, 0, 3, 8}) + ":" + trace);
        System.out.println(floatingTail(new double[] {Double.NaN, Double.NEGATIVE_INFINITY, -0.0, 0.0, 1.0, Double.POSITIVE_INFINITY}) + ":" + trace);
        for (int limit = 0; limit < 5; limit++) {
            trace = 0;
            System.out.println(filteredEffects(limit) + ":" + trace);
            trace = 0;
            System.out.println(matchingTail(new int[] {1, 3}, limit) + ":" + trace);
            trace = 0;
            System.out.println(alternativeTail(limit) + ":" + trace);
            trace = 0;
            System.out.println(largeTail(limit) + ":" + labelledTail(limit) + ":" + trace);
            trace = 0;
            System.out.println(guardedCleanup(limit) + ":" + trace);
            trace = 0;
            System.out.println(postTest(limit) + ":" + nestedPostTest(limit) + ":" + trace);
            trace = 0;
            System.out.println(lazyPostTest(limit) + ":" + trace);
            trace = 0;
            System.out.println(skipTest(limit) + ":" + scopedTest(limit) + ":" + labelledSkip(limit) + ":" + trace);
            trace = 0;
            System.out.println(retainedScopedTest(limit) + ":" + trace);
            trace = 0;
            System.out.println(protectedPostTest(limit) + ":" + trace);
            trace = 0;
            System.out.println(countdown(limit) + ":" + savedCountdown(limit) + ":" + protectedCountdown(limit, new int[] {3, 4}) + ":" + trace);
            System.out.println(boundedIndex(new int[] {-1, 2, 3}, limit) + ":" + boundedFlag(new int[] {-1, 2, 3}, limit) + ":" + cleanupObservedSearch(new int[] {2, 3}, limit, (limit & 1) == 0) + ":" + trace);
            System.out.println(boundedCounterReturn(new int[] {2, 3}, limit));
            trace = 0;
            System.out.println(effectfulSearch(new int[] {1, 3}, limit) + ":" + observedExhaustion(new int[] {1, 3}, limit) + ":" + trace);
            System.out.println(persistentMatch(new int[][] {{1, 1}, {3, 3}, {2, 3}}, limit) + ":" + trace);
        }
        System.out.println(repeatedSearch(new int[] {1, 3}) + ":" + repeatedSearch(new int[0]) + ":" + trace);
        System.out.println(effectfulSearch(new int[0], 0) + ":" + observedExhaustion(null, 0) + ":" + trace);
        System.out.println(wrappingCountdown(-2147483648) + ":" + trace);
        System.out.println(protectedCountdown(2, null) + ":" + trace);
        System.out.println(boundedIndex(new int[0], 0) + ":" + boundedFlag(new int[0], 0) + ":" + cleanupObservedSearch(null, 0, false) + ":" + trace);
        System.out.println(boundedCounterReturn(new int[0], 0));
        char[][] scans = { {'x'}, {' ', '\t', 'y'}, {' ', '\000'}, {}, {' '} };
        for (int i = 0; i < scans.length; i++) {
            trace = 0;
            try { System.out.println("scan:" + readAfterPostTest(scans[i]) + ":" + trace); }
            catch (ArrayIndexOutOfBoundsException e) { System.out.println("scan-end:" + trace); }
        }
        trace = 0;
        try { protectedPostTest(-1); }
        catch (IllegalArgumentException e) { System.out.println("throw:" + trace); }
    }
}
