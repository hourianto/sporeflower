package support;
public final class IndexedEnvelope {
    static {
        Calls.mark("indexed-init");
        if (Calls.mode == 1) throw new RuntimeException();
    }
    public final int sum;
    public IndexedEnvelope(int first, int second, int index) {
        Calls.mark("indexed-constructor");
        if (Calls.mode == 6) throw new RuntimeException();
        sum = first + second + index;
    }
}
