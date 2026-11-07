package typescope;

public class Trace {
    public static int value;
    public static int record(int value) {
        Trace.value = Trace.value * 10 + value;
        return value;
    }
}
