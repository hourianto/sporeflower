public final class QualityLocalUpdates {
    private static int trace;

    private static int mark(int value) {
        trace = trace * 31 + value;
        return value;
    }

    private static int pair(int left, int right) {
        return mark(left * 17 + right);
    }

    private static int effect(boolean fail) {
        mark(71);
        if (fail) {
            throw new IllegalArgumentException();
        }
        return 3;
    }

    public static void padding(int length) {
        int remaining = 512 - length;
        while (true) {
            int old = remaining;
            remaining = remaining - 1;
            if (old <= 0) {
                break;
            }
            mark(0);
        }
    }

    public static int escapingCountdown(int remaining) {
        while (true) {
            int old = remaining;
            remaining = remaining - 1;
            if (old <= 0) {
                break;
            }
            mark(remaining);
        }
        return remaining;
    }

    public static int addNegative(int remaining) {
        while (true) {
            int old = remaining;
            remaining = remaining + -1;
            if (old <= 0) {
                break;
            }
            mark(remaining);
        }
        return remaining;
    }

    public static int iincCountdown(int remaining) {
        while (remaining-- > 0) {
            mark(remaining);
        }
        return remaining;
    }

    public static int stackCountdown(int remaining) {
        while (true) {
            int old = remaining;
            remaining = remaining - 1;
            if (old <= 0) {
                break;
            }
            mark(remaining + 43);
        }
        return remaining;
    }

    public static int wideSlotCountdown(long seed) {
        int remaining = (int) (seed & 3L);
        int stride = mark(7);
        int result = 0;
        while (true) {
            result += stride;
            int old = remaining;
            remaining = remaining - 1;
            if (old < 0) {
                break;
            }
        }
        return pair(result, remaining);
    }

    public static int delayedCountdown(int remaining) {
        int result = 0;
        while (true) {
            int next = remaining - 1;
            if (remaining < 0) {
                break;
            }
            result += mark(next + 43);
            remaining = next;
        }
        return result;
    }

    public static int delayedIncrement(int index, int limit) {
        int result = 0;
        while (index < limit) {
            int next = index + 1;
            result += mark(index);
            index = next;
        }
        return result;
    }

    public static int chainedCursor(boolean includeExtra, int limit) {
        int result = 0;
        int cursor = 0;
        while (cursor < limit) {
            int extra = 0;
            int middle;
            if (includeExtra) {
                middle = cursor + 1;
                extra = mark(cursor);
            } else {
                middle = cursor;
            }
            int next = middle + 1;
            result += pair(extra, pair(mark(middle), mark(next)));
            cursor = next + 1;
        }
        return result;
    }

    public static int callOld(int value) {
        int old = value;
        value = value + 1;
        return pair(old, value);
    }

    public static int repeatedOld(int value) {
        int old = value;
        value = value + 1;
        return pair(old + old, value);
    }

    public static int oldAfterNew(int value) {
        int old = value;
        value = value + 1;
        return pair(value, old);
    }

    public static int olderSnapshot(int value) {
        int old = value;
        value = value + 1;
        mark(value);
        return old;
    }

    public static int handlerObserved(int value, boolean fail) {
        try {
            int old = value;
            value = value + 1;
            return pair(effect(fail), old);
        } catch (IllegalArgumentException failure) {
            return value;
        }
    }

    public static int updateBeforeFailure(int value, boolean fail) {
        try {
            int old = value;
            value = value + 1;
            return pair(old, effect(fail));
        } catch (IllegalArgumentException failure) {
            return value;
        }
    }

    public static int earlyContinue(int remaining) {
        int count = 0;
        while (remaining-- > 0) {
            if ((remaining & 1) == 0) {
                continue;
            }
            count++;
        }
        return count;
    }

    public static int convergenceSentinel(int seed) {
        int initial = mark(seed);
        int term = pair(initial, initial);
        int current = initial;
        int previous = initial + 1;
        int iterations = 0;
        while (current != previous) {
            previous = current;
            current = pair(current, term);
            term = pair(term, initial);
            if (iterations++ > 2) {
                break;
            }
        }
        return pair(previous, current);
    }

    public static int independentLoopNeighbor(int current) {
        int result = 0;
        for (int remaining = 3; remaining > 0; remaining--) {
            int neighbor = current + 1;
            result = pair(result, pair(current, neighbor));
            current = neighbor ^ 17;
        }
        return result;
    }

    public static void main(String[] args) {
        int[] starts = new int[] { -3, 0, 1, 4, Integer.MIN_VALUE, Integer.MAX_VALUE };
        for (int index = 0; index < starts.length; index++) {
            int value = starts[index];
            mark(callOld(value));
            mark(repeatedOld(value));
            mark(oldAfterNew(value));
            mark(olderSnapshot(value));
            mark(handlerObserved(value, false));
            mark(handlerObserved(value, true));
            mark(updateBeforeFailure(value, false));
            mark(updateBeforeFailure(value, true));
            mark(convergenceSentinel(value));
            mark(independentLoopNeighbor(value));
            mark(wideSlotCountdown(value));
            if (value < 5) {
                padding(512 - value);
                mark(escapingCountdown(value));
                mark(addNegative(value));
                mark(iincCountdown(value));
                mark(stackCountdown(value));
                mark(delayedCountdown(value));
            }
        }
        mark(earlyContinue(7));
        mark(delayedIncrement(-2, 5));
        mark(chainedCursor(false, 7));
        mark(chainedCursor(true, 7));
        System.out.println(trace);
    }
}
