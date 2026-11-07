public class SwitchDefaultTargets {
    static int trace;

    static int selector(int value) {
        trace = trace * 3 + 1;
        if (value == -7) {
            throw new IllegalArgumentException();
        }
        return value;
    }

    static int dense(int choice) {
        switch (selector(choice)) {
            case 0:
                trace += 10;
                return 20;
            case 1:
                trace += 20;
            case 2:
            default:
                trace += 30;
                return 30;
            case 5:
                trace += 50;
                return 50;
        }
    }

    static int sparse(int choice) {
        switch (selector(choice)) {
            case -10000:
                trace += 10;
                return 20;
            case 30:
            default:
                trace += 30;
                return 30;
            case 10000:
                trace += 50;
                return 50;
        }
    }

    static int allDefault(int choice) {
        switch (selector(choice)) {
            case 6:
            case 9:
            default:
                trace += 40;
                return -choice;
        }
    }

    static int emptyDefault(int choice) {
        switch (selector(choice)) {
            case 11:
            case 12:
            default:
                break;
        }
        return trace;
    }

    static int mergedReturns(int choice) {
        switch (selector(choice)) {
            case -10000:
                return 7;
            case 0:
                return 7;
            case 1:
                return 9;
            default:
                return 7;
        }
    }

    static int defaultFirst(int choice) {
        switch (selector(choice)) {
            default:
                return 7;
            case -10000:
                return 7;
            case 0:
                return 7;
            case 1:
                return 9;
        }
    }

    static int cleanup(int choice) {
        try {
            return 100 / choice;
        } finally {
            switch (selector(choice)) {
                case 0:
                    trace += 10;
                    break;
                case 1:
                    trace += 20;
                case 2:
                default:
                    trace += 30;
                    break;
                case 5:
                    trace += 50;
                    break;
            }
        }
    }

    static void result(int answer) {
        System.out.print(answer);
        System.out.print(':');
        System.out.println(trace);
    }

    public static void main(String[] args) {
        int[] choices = {-10000, -8, -7, -1, 0, 1, 2, 3, 4, 5, 6, 9, 11, 12, 30, 10000};
        for (int i = 0; i < choices.length; i++) {
            int choice = choices[i];
            trace = choice;
            try { result(dense(choice)); }
            catch (RuntimeException failure) { result(-99); }
            trace = choice;
            try { result(sparse(choice)); }
            catch (RuntimeException failure) { result(-99); }
            trace = choice;
            try { result(allDefault(choice)); }
            catch (RuntimeException failure) { result(-99); }
            trace = choice;
            try { result(emptyDefault(choice)); }
            catch (RuntimeException failure) { result(-99); }
            trace = choice;
            try { result(mergedReturns(choice)); }
            catch (RuntimeException failure) { result(-99); }
            trace = choice;
            try { result(defaultFirst(choice)); }
            catch (RuntimeException failure) { result(-99); }
            trace = choice;
            try { result(cleanup(choice)); }
            catch (RuntimeException failure) { result(-99); }
        }
    }
}
