public class ReconvergingSwitches {
    static StringBuffer trace;
    static int failure;

    static int mark(int value) {
        trace.append(value).append(',');
        if (failure == value) throw new IllegalArgumentException();
        return value;
    }

    static int run(int selector, int guard) {
        int result = 0;
        switch (selector) {
            case 0:
                result += mark(10);
            case 1:
                if (guard < 0) return mark(11);
                result += mark(12);
                break;
            case 2:
                result += mark(20);
            case 3:
                for (int index = 0; index < guard; index++) result += mark(21);
                break;
            case 4:
                return mark(40);
            default:
                result += mark(50);
        }
        if (guard == 0) result += mark(60);
        else result += mark(61);
        return result + mark(70);
    }

    static int protectedRun(int selector, int guard) {
        try {
            mark(1);
            int result = 0;
            switch (selector) {
                case 0:
                    result += mark(10);
                case 1:
                    if (guard < 0) return mark(11);
                    result += mark(12);
                    break;
                case 2:
                    result += mark(20);
                case 3:
                    result += mark(21);
                    break;
                case 4:
                    return mark(40);
                default:
                    result += mark(50);
            }
            if (guard == 0) result += mark(60);
            else result += mark(61);
            return result + mark(70);
        } catch (IllegalArgumentException exception) {
            return mark(90);
        }
    }

    static int sharedCalculation(int value) {
        switch (value) {
            case 0: value = 65; break;
            case 1: value = 70; break;
            case 2: value = mark(20); break;
            case 3: value = 100; break;
            default: mark(50); value = 63;
        }
        return value - 32;
    }

    public static void main(String[] args) {
        int[] failures = {-1, 1, 10, 11, 12, 20, 21, 40, 50, 60, 61, 70};
        for (int mode = 0; mode < 2; mode++) {
            for (int selector = -1; selector <= 5; selector++) {
                for (int guard = -1; guard <= 2; guard++) {
                    for (int index = 0; index < failures.length; index++) {
                        trace = new StringBuffer();
                        failure = failures[index];
                        int result;
                        try {
                            result = mode == 0 ? run(selector, guard) : protectedRun(selector, guard);
                        } catch (IllegalArgumentException exception) {
                            result = -1;
                        }
                        System.out.println(mode + ":" + selector + ":" + guard + ":" + failure + ":" + result + ":" + trace);
                    }
                }
            }
        }
        for (int selector = -1; selector <= 4; selector++) {
            for (int index = 0; index < failures.length; index++) {
                trace = new StringBuffer();
                failure = failures[index];
                try { System.out.println("shared:" + selector + ":" + sharedCalculation(selector) + ":" + trace); }
                catch (IllegalArgumentException e) { System.out.println("shared-throw:" + selector + ":" + trace); }
            }
        }
    }
}
