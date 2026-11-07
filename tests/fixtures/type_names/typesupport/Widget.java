package typesupport;

public class Widget {
    public static int total = typescope.Trace.record(1);
    public int value;

    public Widget(int value) { this.value = value; }
    public static int read() { return total; }
}
