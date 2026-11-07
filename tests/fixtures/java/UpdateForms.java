public class UpdateForms {
    static int trace;
    static int effect(int value) { trace = trace * 31 + value; return value; }

    static int counted(int limit) {
        int result = 0;
        for (int i = 0; i < limit; i++) {
            if (i == 3) continue;
            result += effect(i);
        }
        return result;
    }

    static int earlierEqualUpdate(int limit) {
        int result = 0;
        for (int i = 0; i < limit; i++) {
            i++;
            if (i == 3) continue;
            result += effect(i);
        }
        return result;
    }

    static int bypassedTail(int limit) {
        int i = 0;
        int result = 0;
        while (i < limit) {
            i++;
            if (i == 3) continue;
            result += effect(i);
            i += 2;
        }
        return result;
    }

    static long updates(int value) {
        int first = value;
        first += (first = effect(7));
        first = first + -1;
        long wide = first;
        wide <<= 40;
        wide ^= 0x123456789abcdef0L;
        wide >>>= value;
        return wide;
    }

    static int bodyScopedStep(int limit) {
        int index = 0;
        int sum = 0;
        while (index < limit) {
            int step = effect(index % 3 + 1);
            sum += index;
            index += step;
        }
        return sum;
    }

    static int usedAfterLoop(int limit) {
        int index;
        for (index = 0; index < limit; index++) effect(index);
        return index;
    }

    static int dynamicContinuation(int limit) {
        int step = 1;
        int result = 0;
        for (int index = 0; index < limit; index += step) {
            step = index % 3 + 1;
            if (index == 3) continue;
            if (index == 7) break;
            result += effect(index);
        }
        return result;
    }

    static int nestedContinuation(int limit) {
        int result = 0;
        outer: for (int index = 0; index < limit; index++) {
            for (int inner = 0; inner < 3; inner++) {
                if (inner == 1 && index % 2 == 0) continue outer;
                if (index == 5) break outer;
                result += effect(index * 3 + inner);
            }
        }
        return result;
    }

    static int finallyBeforeContinuation(int limit) {
        int index = 0;
        for (; index < limit; index++) {
            try {
                if (index % 2 == 0) continue;
                if (index == 7) return index;
                effect(index);
            } finally {
                effect(100 + index);
            }
        }
        return index;
    }

    public static void main(String[] args) {
        for (int limit = 0; limit < 12; limit++) {
            trace = 0;
            System.out.println(new StringBuffer().append(counted(limit)).append(':').append(earlierEqualUpdate(limit)).append(':').append(bypassedTail(limit)).append(':').append(trace).toString());
            System.out.println(new StringBuffer().append(updates(limit)).append(':').append(trace).toString());
            System.out.println(new StringBuffer().append(bodyScopedStep(limit)).append(':').append(trace).toString());
            System.out.println(new StringBuffer().append(usedAfterLoop(limit)).append(':').append(trace).toString());
            System.out.println(new StringBuffer().append(dynamicContinuation(limit)).append(':').append(trace).toString());
            System.out.println(new StringBuffer().append(nestedContinuation(limit)).append(':').append(trace).toString());
            System.out.println(new StringBuffer().append(finallyBeforeContinuation(limit)).append(':').append(trace).toString());
        }
        System.out.println(updates(Integer.MIN_VALUE));
        System.out.println(updates(Integer.MAX_VALUE));
    }
}
