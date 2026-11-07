package named;

public class Probe {
    public static int run(int value) { return x.read() + value; }
    static x sameType(x value) { return value; }
}

class x {
    static int value = initialize();
    static int initialize() {
        System.out.println("initialize named.x");
        return 11;
    }
    static int read() { return value; }
}
