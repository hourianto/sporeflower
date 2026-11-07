package support;
public final class Calls {
    public static int mode;
    public static String trace = "";
    public static void mark(String label) { trace = trace + label + ";"; }
    public static String title() {
        mark("title");
        if (mode == 2) throw new RuntimeException();
        return "title";
    }
    public static Sub child() {
        mark("child");
        if (mode == 3) return null;
        if (mode == 4) throw new IllegalArgumentException();
        return new Sub();
    }
}
