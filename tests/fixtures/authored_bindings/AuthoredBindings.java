public class AuthoredBindings {
    final int total;
    static final int bound;
    static int state;

    static {
        int computed = x.read();
        bound = computed * computed;
    }

    AuthoredBindings(int value) { total = value; }
    int sameFinal(int value) { return total + value; }
    static int sameField(int value) { return bound + value; }
    static int sameType(int value) { return x.read() + value; }
    static int sameOwner(int value) { return AuthoredBindings.sameType(value); }
    static int sameLibrary(int value, int root) {
        return Integer.parseInt("7") + value + root;
    }
    static int initializeTarget(int value) {
        state++;
        Initialized item = new Initialized(value);
        return state * 100 + item.value;
    }
    static int choose(byte value) { return 100 + value; }
    static int choose(int value) { return 200 + value; }
    static int conditional(boolean flag, byte value, int ignored) {
        return choose((byte) (flag ? value : x.other));
    }

    public static void main(String[] arguments) {
        System.out.println(new AuthoredBindings(3).sameFinal(4));
        System.out.println(sameField(6));
        System.out.println(sameType(8));
        System.out.println(sameOwner(9));
        System.out.println(sameLibrary(2, 3));
        System.out.println(initializeTarget(2));
        System.out.println(initializeTarget(3));
        System.out.println(conditional(true, (byte) 7, 0));
        System.out.println(conditional(false, (byte) 7, 0));
        System.out.println(named.Probe.run(4));
        System.out.println(named.Probe.run(5));
    }
}

class x {
    static final int fixed = 1;
    static int other = 1;
    static int value = initialize();
    static int initialize() {
        System.out.println("initialize x");
        return 5;
    }
    static int read() { return value; }
}

class Initialized {
    int value;
    static { AuthoredBindings.state = 20; }
    Initialized(int value) { this.value = value; }
}
