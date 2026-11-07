public class AllocationOuter {
    static {
        AllocationState.trace += "outer-init;";
        if (AllocationState.mode == 1) throw new IllegalStateException("outer init");
    }
    public final int value;
    public AllocationOuter(AllocationInner inner) {
        AllocationState.trace += "outer-construct;";
        if (AllocationState.mode == 5) throw new IllegalStateException("outer constructor");
        value = inner.value;
    }
}
