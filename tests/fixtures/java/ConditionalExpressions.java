public class ConditionalExpressions {
    static long trace;
    static int fail;
    static final RuntimeException failure = new RuntimeException("chosen");

    static int number(int value) {
        trace = trace * 31 + value;
        return value;
    }

    static int checked(int value) {
        number(value);
        if (fail == value) throw failure;
        return value;
    }

    static int combine(int first, int second, int third) {
        number(9);
        return first * 100 + second * 10 + third;
    }

    static int calls(boolean condition) {
        return condition ? checked(1) : checked(2);
    }

    static int ordered(boolean condition) {
        return combine(number(1), condition ? checked(2) : checked(3), number(4));
    }

    static int conditionEffect(boolean condition) {
        return number(condition ? 1 : 0) == 0 ? checked(2) : checked(3);
    }

    static boolean selectedTruth(int choice, int raw) {
        return (number(choice) > 0 ? checked(raw) : 0) != 0;
    }

    static boolean selectedZero(int choice, int raw) {
        return (number(choice) > 0 ? checked(raw) : 0) == 0;
    }

    static boolean selectedNonzeroArms(int choice) {
        return (number(choice) > 0 ? 2 : 4) != 0;
    }

    static float floating(float value) {
        number(7);
        return value;
    }

    static boolean selectedFloatEquality(boolean choice, float first, float second) {
        return (choice ? (floating(first) == floating(second) ? 4 : 0) : 0) == 0;
    }

    static int preceding(boolean condition) {
        int value = number(1);
        return condition ? combine(value, checked(2), 0) : checked(3);
    }

    static int following(boolean condition) {
        int value = condition ? checked(1) : checked(2);
        number(3);
        return value;
    }

    static int nested(boolean first, boolean second) {
        return first ? combine(number(1), second ? checked(2) : checked(3), number(4)) : checked(5);
    }

    static int sharedAlternatives(boolean selected, boolean choice, int input) {
        int result;
        if (selected) {
            int shift = input + 1;
            int base = shift * 3;
            result = choice ? combine(base, shift, 1) : combine(base, shift, 2);
        } else {
            result = 0;
        }
        return result;
    }

    static int sharedAfterChoice(boolean selected, boolean choice, int input) {
        int result;
        if (selected) {
            int shift = input + 1;
            int base = shift * 3;
            int part = choice ? combine(base, shift, 1) : 0;
            result = combine(part, base, shift);
        } else {
            result = 0;
        }
        return result;
    }

    static int shared(boolean condition) {
        int result;
        if (condition) {
            int shared = checked(6);
            result = combine(shared, shared, number(7));
        } else {
            result = checked(8);
        }
        return result;
    }

    static int reordered(boolean condition) {
        int result;
        if (condition) {
            int first = number(1);
            int second = number(2);
            result = combine(second, first, 0);
        } else {
            result = checked(3);
        }
        return result;
    }

    static int reused(boolean condition) {
        int result = condition ? checked(1) : checked(2);
        return combine(result, number(3), result);
    }

    static int caught(boolean condition) {
        int saved = number(7);
        try {
            return condition ? checked(1) : checked(2);
        } catch (RuntimeException caught) {
            if (caught != failure) throw caught;
            return saved;
        }
    }

    static int catchMutation(boolean condition) {
        int saved = 0;
        try {
            int result;
            if (condition) {
                saved = number(6);
                result = checked(1);
            } else {
                saved = number(7);
                result = checked(2);
            }
            return result;
        } catch (RuntimeException caught) {
            if (caught != failure) throw caught;
            return saved;
        }
    }

    static Object allocation(boolean condition) {
        return condition ? new ConditionalLeft(number(1)) : new ConditionalRight(number(2));
    }

    static int guarded(boolean condition, int[] values, int divisor) {
        return condition ? values[number(0)] / divisor : checked(2);
    }

    static void report(int result) {
        System.out.println(result);
        System.out.println(trace);
        trace = 0;
    }

    public static void main(String[] args) {
        for (int choice = 0; choice < 2; choice++) {
            boolean condition = choice != 0;
            for (fail = 0; fail < 3; fail++) {
                trace = 0;
                try { report(calls(condition)); } catch (RuntimeException caught) { report(caught == failure ? -1 : -2); }
                try { report(ordered(condition)); } catch (RuntimeException caught) { report(caught == failure ? -1 : -2); }
                try { report(conditionEffect(condition)); } catch (RuntimeException caught) { report(caught == failure ? -1 : -2); }
                for (int raw = -2; raw <= 2; raw++) {
                    try { report(selectedTruth(choice, raw) ? 1 : 0); } catch (RuntimeException caught) { report(caught == failure ? -1 : -2); }
                    try { report(selectedZero(choice, raw) ? 1 : 0); } catch (RuntimeException caught) { report(caught == failure ? -1 : -2); }
                }
                report(selectedNonzeroArms(choice) ? 1 : 0);
                report(selectedFloatEquality(condition, Float.NaN, Float.NaN) ? 1 : 0);
                report(selectedFloatEquality(condition, Float.POSITIVE_INFINITY, Float.POSITIVE_INFINITY) ? 1 : 0);
                report(selectedFloatEquality(condition, -0.0F, 0.0F) ? 1 : 0);
                try { report(preceding(condition)); } catch (RuntimeException caught) { report(caught == failure ? -1 : -2); }
                try { report(following(condition)); } catch (RuntimeException caught) { report(caught == failure ? -1 : -2); }
                for (int other = 0; other < 2; other++) {
                    try { report(nested(condition, other != 0)); } catch (RuntimeException caught) { report(caught == failure ? -1 : -2); }
                    report(sharedAlternatives(condition, other != 0, 5));
                    report(sharedAfterChoice(condition, other != 0, 5));
                }
                report(caught(condition));
                report(catchMutation(condition));
            }
            fail = 0;
            report(shared(condition));
            report(reordered(condition));
            report(reused(condition));
            report(allocation(condition) instanceof ConditionalLeft ? 1 : 2);
            report(guarded(condition, new int[] {12}, 2));
            try { report(guarded(condition, null, 0)); } catch (NullPointerException caught) { report(-3); }
            try { report(guarded(condition, new int[] {12}, 0)); } catch (ArithmeticException caught) { report(-4); }
            ConditionalMessage message = new ConditionalMessage(condition ? null : "input", condition ? null : new ConditionalPosition(), condition ? null : failure);
            System.out.println(message.getMessage());
            report(message.cause == null ? 0 : 1);
        }
    }
}

class ConditionalMessage extends RuntimeException {
    Throwable cause;
    ConditionalMessage(String message, ConditionalPosition position, Throwable cause) {
        super(new StringBuffer().append(message == null ? "" : new StringBuffer().append(message).append(" ").toString())
            .append(position == null ? "" : new StringBuffer().append("(position:").append(position.position()).append(") ").toString())
            .append(cause == null ? "" : new StringBuffer().append("caused by: ").append(cause).toString()).toString());
        this.cause = cause;
    }
}

class ConditionalPosition {
    int position() { return ConditionalExpressions.number(5); }
}

class ConditionalLeft {
    static int initialized = ConditionalExpressions.number(6);
    ConditionalLeft(int value) { ConditionalExpressions.number(value + 10); }
}

class ConditionalRight {
    static int initialized = ConditionalExpressions.number(7);
    ConditionalRight(int value) { ConditionalExpressions.number(value + 10); }
}
