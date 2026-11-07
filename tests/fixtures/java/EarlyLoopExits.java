public class EarlyLoopExits {
    static int trace;
    static int mark(int value) { trace = trace * 31 + value; return value; }

    static int earlyReturn(int bound, int stop) {
        int sum = 0;
        for (int i = 0; i < bound; i++) {
            mark(i);
            if (i == stop) return mark(sum + 100);
            sum += i;
        }
        return mark(sum + 300);
    }

    static int terminalTail(int bound, int stop) {
        int sum = 0;
        for (int i = 0; i < bound; i++) {
            mark(i);
            if (i == stop) {
                mark(80);
                if ((sum & 1) == 0) return mark(sum + 100);
                return mark(sum + 200);
            }
            sum += i;
        }
        return mark(sum + 300);
    }

    static int earlyThrow(int bound, int stop) {
        int sum = 0;
        for (int i = 0; i < bound; i++) {
            mark(i);
            if (i == stop) { mark(90); throw new IllegalStateException("hit"); }
            sum += i;
        }
        return mark(sum);
    }

    static int nested(int bound, int stop) {
        int sum = 0;
        for (int i = 0; i < bound; i++) {
            for (int j = 0; j < i; j++) {
                mark(i * 10 + j);
                if (j == stop) return mark(sum + 400);
                sum += i + j;
            }
        }
        return mark(sum + 500);
    }

    public static void main(String[] args) {
        for (int bound = 0; bound < 7; bound++) {
            for (int stop = -1; stop < 7; stop++) {
                trace = 0;
                System.out.println(earlyReturn(bound, stop) + ":" + trace);
                trace = 0;
                System.out.println(terminalTail(bound, stop) + ":" + trace);
                trace = 0;
                try { System.out.println(earlyThrow(bound, stop) + ":" + trace); }
                catch (RuntimeException failure) { System.out.println(failure.getMessage() + ":" + trace); }
                trace = 0;
                System.out.println(nested(bound, stop) + ":" + trace);
            }
        }
    }
}
