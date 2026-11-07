public class AllocationInner {
    static {
        AllocationState.trace += "inner-init;";
        if (AllocationState.mode == 2) throw new IllegalStateException("inner init");
    }
    public final int value;
    public AllocationInner(int tag, int value) {
        AllocationState.trace += "inner-construct;";
        if (AllocationState.mode == 4) throw new IllegalStateException("inner constructor");
        this.value = tag + value;
    }
}
