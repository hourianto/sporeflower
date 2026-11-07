package support;
public final class Sub extends Base {
    static { Calls.mark("sub-init"); }
    public Sub() {
        Calls.mark("sub");
        if (Calls.mode == 5) throw new IllegalArgumentException();
    }
}
