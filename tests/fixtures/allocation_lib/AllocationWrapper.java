public class AllocationWrapper {
    static {
        AllocationState.trace += "initialize;";
        if (AllocationState.mode == 1) throw new IllegalStateException("initialization");
    }
    public final int value;
    public AllocationWrapper(AllocationResource resource, int marker) {
        AllocationState.trace += "construct;";
        if (AllocationState.mode == 3) throw new IllegalArgumentException("constructor");
        value = resource.id + marker;
    }
}
