public class CoalescingProbe {
    static int calls;
    static int failAt;

    static int next(int value) {
        if (++calls == failAt) throw new IllegalArgumentException();
        return value * 3 + calls;
    }

    static int snapshots(boolean choose) {
        int saved = 7;
        int current = 11;
        try {
            current = next(current);
            saved = current;
            current = next(choose ? (saved = next(saved)) : saved);
            saved = current;
            return saved + next(current);
        } catch (IllegalArgumentException expected) {
            return saved * 1000 + current;
        }
    }

    static int iterations(int count) {
        int left = 1;
        int right = 2;
        int saved = 0;
        while (count-- > 0) {
            saved = left;
            left = right;
            right = saved + right;
            if ((count & 1) == 0) left += 3;
        }
        return left * 10000 + right * 100 + saved;
    }

    static class Pair {
        int result;
        Pair(int left, int right) {
            result = next(left) + right;
        }
    }

    static int captures(boolean choose) {
        int saved = 5;
        try {
            Pair value = new Pair(next(saved = next(saved)),
                choose ? (saved = next(saved)) : saved);
            return value.result + saved;
        } catch (IllegalArgumentException expected) {
            return -saved;
        }
    }

    static String nestedCatches(boolean choose) {
        RuntimeException result;
        try {
            throw new RuntimeException("outer");
        } catch (RuntimeException outer) {
            if (choose) {
                try {
                    throw new RuntimeException("inner");
                } catch (RuntimeException inner) {
                    result = inner;
                }
            } else {
                result = outer;
            }
        }
        return result.getMessage();
    }

    public static void main(String[] args) {
        for (int failure = 0; failure < 8; failure++) {
            failAt = failure;
            for (int choice = 0; choice < 2; choice++) {
                calls = 0;
                System.out.println(snapshots(choice == 0) + ":" + calls);
                calls = 0;
                System.out.println(captures(choice == 0) + ":" + calls);
            }
        }
        for (int count = 0; count < 12; count++) {
            System.out.println(iterations(count));
        }
        System.out.println(nestedCatches(false));
        System.out.println(nestedCatches(true));
    }
}
