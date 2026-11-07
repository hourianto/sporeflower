public class ConstructorCaptures {
    static int state;

    static class Before {
        int value;
        static { state = 10; }
        Before(int value) { this.value = value; }
    }

    static class Shared {
        int value;
        static { state = 20; }
        Shared(int first, int second) { value = first * 100 + second; }
    }

    static class Nested {
        int value;
        static { state = 30; }
        Nested(int[] values) { value = values[0] * 100 + values[1]; }
    }

    static class Conditional {
        String value;
        static { state = 40; }
        Conditional(String value) { this.value = value; }
    }

    static class ThrowCondition {
        static { state = 50; }
        ThrowCondition(String value) { }
    }

    static int read() { return ++state; }

    static int before() {
        int saved = read();
        return new Before(saved).value * 100 + state;
    }

    static int shared() {
        int saved;
        Shared node = new Shared(saved = read(), saved);
        return node.value * 100 + state;
    }

    static int nested() {
        Nested node = new Nested(new int[] { read(), read() });
        return node.value * 100 + state;
    }

    static int conditional(int threshold) {
        Conditional value = new Conditional(++state > threshold ? "yes" : "no");
        return state * 100 + value.value.length();
    }

    static boolean throwCondition() { state++; throw new RuntimeException(); }

    static int conditionalFailure() {
        try {
            new ThrowCondition(throwCondition() ? "yes" : "no");
            return -1;
        } catch (RuntimeException expected) {
            return state;
        }
    }

    public static void main(String[] args) {
        System.out.println(before());
        System.out.println(shared());
        System.out.println(nested());
        System.out.println(conditional(40));
        System.out.println(conditional(100));
        System.out.println(conditionalFailure());
    }
}
