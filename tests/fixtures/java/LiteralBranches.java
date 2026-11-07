import java.io.IOException;

public class LiteralBranches {
    static int trace;
    static int mark(int value) { trace = trace * 31 + value; return value; }

    static String branches() {
        int gate = 1;
        mark(1);
        Object value;
        if (gate == 1) value = "live";
        else value = new byte[mark(99)];
        Object absent = null;
        if (absent != null) mark(98);
        else mark(2);
        if (gate != 1) {
            try { throw new IOException("dead"); }
            catch (IOException failure) { mark(97); }
        }
        switch (gate) {
            case 0: mark(96); break;
            case 1: mark(3); break;
            default: mark(95);
        }
        // A heap read remains observable, including its class initialization.
        if (LiteralBranchOwner.value == 1) mark(4);
        return (String) value;
    }

    static int forever() {
        int left = -1;
        int right = -1;
        while (left == right) mark(5);
        return mark(94);
    }

    public static void main(String[] args) {
        System.out.println(branches() + ":" + trace);
        // forever is compiled on both sides, but intentionally never executed.
    }
}

class LiteralBranchOwner {
    static int value = 1;
    static { LiteralBranches.mark(6); }
}
