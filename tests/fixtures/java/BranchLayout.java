public class BranchLayout {
    static int trace;

    static void hit(int value) { trace = trace * 31 + value; }

    static boolean test(int tag, int input) {
        hit(tag);
        if (input == tag + 10) throw new IllegalArgumentException();
        return (input & (1 << (tag - 1))) != 0;
    }

    static void twoArm(int value) {
        if (value == 5) hit(41);
        else hit(43);
    }

    static void negated(int value) {
        if (value != 5) hit(47);
        else hit(53);
    }

    static void nestedThen(int state, int command) {
        if (state == 1) {
            if (command == 2) hit(151);
        } else {
            hit(157);
        }
    }

    static void nestedDispatch(int state, int command) {
        if (state == 1) {
            if (command == 1) hit(59);
            else if (command == 2) hit(61);
        } else if (state == 2) {
            if (command == 1) hit(67);
            else if (command == 2) hit(71);
        } else if (state == 3) {
            if (command == 1) hit(73);
            else if (command == 2) hit(79);
        } else if (state == 4) {
            if (command == 1) hit(83);
            else if (command == 2) hit(89);
        }
    }

    static void compound(int input) {
        if (test(1, input) && (test(2, input) || test(3, input))) {
            hit(97);
        } else {
            hit(101);
        }
    }

    static void protectedArms(int value) {
        if (value == 0) {
            try { hit(12 / value); }
            catch (ArithmeticException failure) { hit(103); }
        } else {
            try { hit(24 / value); }
            finally { hit(107); }
        }
    }

    static void guard(int value) {
        if (value == 0) throw new IllegalArgumentException();
        else hit(109);
    }

    static void typedDispatch(Object value) {
        if (value instanceof String) hit(163);
        else if (value instanceof Integer) hit(167);
        else if (value instanceof Long) hit(173);
        else throw new IllegalArgumentException("type");
    }

    static void floating(double left, double right) {
        if (left < right) hit(113);
        else hit(127);
        if (left >= right) hit(131);
        else hit(137);
    }

    public static void main(String[] args) {
        for (int input = -1; input < 16; input++) {
            trace = 0;
            twoArm(input);
            negated(input);
            for (int command = 0; command < 4; command++) {
                nestedDispatch(input, command);
                nestedThen(input, command);
            }
            try { compound(input); }
            catch (IllegalArgumentException failure) { hit(139); }
            protectedArms(input);
            try { guard(input); }
            catch (IllegalArgumentException failure) { hit(149); }
            System.out.println(trace);
        }
        double[] values = { -1.0, -0.0, 0.0, 1.0, Double.NaN,
                Double.NEGATIVE_INFINITY, Double.POSITIVE_INFINITY };
        for (int a = 0; a < values.length; a++) {
            for (int b = 0; b < values.length; b++) {
                trace = 0;
                floating(values[a], values[b]);
                System.out.println(trace);
            }
        }
        Object[] types = { null, "text", new Integer(1), new Long(2), new Object() };
        for (int index = 0; index < types.length; index++) {
            trace = 0;
            try { typedDispatch(types[index]); }
            catch (IllegalArgumentException failure) { hit(179); }
            System.out.println(trace);
        }
    }
}
