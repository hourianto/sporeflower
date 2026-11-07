public class QualitySearch {
    static int trace;
    static int selected;

    static void mark(int value) {
        trace = trace * 31 + value;
    }

    static int candidate(int[] values, int index) {
        mark(index + 1);
        return values[index];
    }

    static void choose(int value) {
        mark(value);
        selected = value;
    }

    static boolean probe(int mask, int bit) {
        mark(bit);
        if (mask < 0) throw new IllegalArgumentException();
        return (mask & bit) != 0;
    }

    static void oneShotChoice(int mask) {
        int counter = 1;
        boolean rejected = true;
        for (; true; counter++) {
            if (counter > 1) {
                rejected = false;
                break;
            }
            if (probe(mask, 1) || probe(mask, 2) || probe(mask, 4)) continue;
            break;
        }
        if (!rejected) mark(401);
        mark(402);
    }

    static void exhaustionEffects(int mask) {
        int counter = 7;
        boolean rejected = true;
        for (; true; counter += 3) {
            if (counter != 7) {
                mark(411);
                rejected = false;
                break;
            }
            mark(412);
            if (probe(mask, 1)) continue;
            mark(413);
            break;
        }
        if (!rejected) mark(414);
        mark(415);
    }

    static void wrappedChoice(int mask) {
        int counter = 2147483647;
        boolean rejected = true;
        for (; true; counter++) {
            if (counter < 0) {
                rejected = false;
                break;
            }
            if (probe(mask, 1)) continue;
            break;
        }
        if (!rejected) mark(421);
    }

    static void observedCounter(int mask) {
        int counter = 1;
        boolean rejected = true;
        for (; true; counter++) {
            if (counter > 1) {
                rejected = false;
                break;
            }
            mark(counter);
            if (probe(mask, 1)) continue;
            break;
        }
        if (!rejected) mark(431);
    }

    static void twoTrips(int mask) {
        int counter = 1;
        boolean rejected = true;
        for (; true; counter++) {
            if (counter > 2) {
                rejected = false;
                break;
            }
            if (probe(mask, 1)) continue;
            break;
        }
        if (!rejected) mark(441);
    }

    static void protectedRepeat(int mask) {
        int counter = 1;
        boolean rejected = true;
        for (; true; counter++) {
            if (counter > 1) {
                rejected = false;
                break;
            }
            try {
                if (probe(mask, 1)) continue;
                break;
            } finally {
                mark(451);
            }
        }
        if (!rejected) mark(452);
    }

    static void effectfulBound(int mask) {
        int counter = 1;
        boolean rejected = true;
        for (; true; counter++) {
            mark(461);
            if (counter > 1) {
                rejected = false;
                break;
            }
            if (probe(mask, 1)) continue;
            break;
        }
        if (!rejected) mark(462);
    }

    static int fallbackSearch(int[] preferred, int[] fallback, int wanted) {
        search: {
            for (int index = 0; index < preferred.length; index++) {
                int value = candidate(preferred, index);
                if (value == wanted) {
                    choose(value);
                    mark(101);
                    break search;
                }
            }
            mark(201);
            for (int index = 0; index < fallback.length; index++) {
                int value = candidate(fallback, index);
                if (value == wanted) {
                    choose(value);
                    mark(102);
                    break search;
                }
            }
        }
        mark(301);
        return selected;
    }

    static int nestedFallback(int[][] preferred, int[][] fallback, int wanted) {
        for (int row = 0; row < preferred.length; row++) {
            search: {
                for (int index = 0; index < preferred[row].length; index++) {
                    int value = candidate(preferred[row], index);
                    if (value == wanted) {
                        choose(value);
                        mark(111);
                        break search;
                    }
                }
                mark(211);
                for (int index = 0; index < fallback[row].length; index++) {
                    int value = candidate(fallback[row], index);
                    if (value == wanted) {
                        choose(value);
                        mark(112);
                        break search;
                    }
                }
            }
            mark(311);
        }
        return selected;
    }

    static int chainedFallback(int[] first, int[] second, int[] third, int wanted) {
        search: {
            for (int index = 0; index < first.length; index++) {
                int value = candidate(first, index);
                if (value == wanted) {
                    choose(value);
                    mark(121);
                    break search;
                }
            }
            mark(221);
            for (int index = 0; index < second.length; index++) {
                int value = candidate(second, index);
                if (value == wanted) {
                    choose(value);
                    mark(122);
                    break search;
                }
            }
            mark(222);
            for (int index = 0; index < third.length; index++) {
                int value = candidate(third, index);
                if (value == wanted) {
                    choose(value);
                    mark(123);
                    break search;
                }
            }
        }
        mark(321);
        return selected;
    }
}
