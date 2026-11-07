import java.io.IOException;

public class FinalJoin {
    public Object value; // The regression patches this field's classfile flag to final.
    static FinalJoin escaped;
    static String trace;

    FinalJoin(int mode) throws Throwable {
        escaped = this;
        try {
            value = choose(mode);
        } catch (IOException failure) {
            value = mark("fallback");
        } catch (Throwable failure) {
            value = mark("abrupt");
            throw failure;
        }
        mark("after");
    }

    FinalJoin(boolean choice) {
        escaped = this;
        if (choice) {
            value = mark("yes");
            observe();
        } else {
            value = mark("no");
            observe();
        }
    }

    static Object choose(int mode) throws IOException {
        mark("choose");
        if (mode == 1) throw new IOException("checked");
        if (mode == 2) throw new IllegalStateException("runtime");
        if (mode == 3) throw new AssertionError("error");
        return mark("normal");
    }

    static Object mark(String value) { trace += value + ","; return value; }
    void observe() { trace += "seen=" + value + ","; }

    public static void main(String[] args) {
        for (int mode = 0; mode < 4; mode++) {
            trace = "";
            try { new FinalJoin(mode); }
            catch (Throwable failure) { trace += failure.getMessage() + ","; }
            System.out.println(trace + escaped.value);
        }
        for (int choice = 0; choice < 2; choice++) {
            trace = "";
            new FinalJoin(choice == 1);
            System.out.println(trace + escaped.value);
        }
    }
}
