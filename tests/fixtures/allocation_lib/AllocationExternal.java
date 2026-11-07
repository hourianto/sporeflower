/** Compiled separately and supplied as a library, so the decompiler cannot add triggers here. */
public class AllocationExternal {
    static {
        AllocationState.trace += "initialize;";
        if (AllocationState.mode == 1) throw new IllegalStateException("initialization");
    }
    public final int value;
    public AllocationExternal(int value) {
        AllocationState.trace += "construct;";
        if (AllocationState.mode == 3) throw new IllegalArgumentException("constructor");
        this.value = value;
    }
}
