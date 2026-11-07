class FinalDelegationTrace {
    static String text = "";
    static int fail;
}
class FinalDelegationBase {
    FinalDelegationBase() { FinalDelegationTrace.text += "super:" + observed() + ";"; }
    FinalDelegationBase(int first, int second) {
        FinalDelegationTrace.text += "super:" + first + ":" + second + ":" + observed() + ";";
    }
    int observed() { return -1; }
}
public class ConstructorFinalDelegation extends FinalDelegationBase {
    // Patched to final after javac: legal JVM constructors may leave its VM
    // default in one path and assign it after this-delegation in another.
    int value;
    int other;
    boolean conditional;
    ConstructorFinalDelegation(boolean ignored) {
        super();
        if (constantZero() != 0) FinalDelegationTrace.text += "unreachable;";
        FinalDelegationTrace.text += "body;";
        this.other = 5;
    }
    ConstructorFinalDelegation(int first, int second, boolean ignored) {
        super(second, first);
        FinalDelegationTrace.text += "body;";
        this.other = second;
    }
    ConstructorFinalDelegation(int value) {
        this(argument(1) != 0);
        this.value = value;
    }
    ConstructorFinalDelegation(int value, int unused) {
        this(argument(1), argument(2), false);
        this.value = value;
    }
    ConstructorFinalDelegation(String ignored, boolean choose) {
        if (choose) {
            this.conditional = true;
            FinalDelegationTrace.text += "true:" + this.conditional + ";";
        } else {
            FinalDelegationTrace.text += "false:" + this.conditional + ";";
        }
        FinalDelegationTrace.text += "join:" + this.conditional + ";";
    }
    ConstructorFinalDelegation(int[] values, int index) {
        super();
        this.value = values[index++] + index;
    }
    ConstructorFinalDelegation(int[] values) {
        this(values, 0);
        this.other = 9;
    }
    ConstructorFinalDelegation(boolean choose, boolean fail) {
        super();
        FinalDelegationTrace.text += "branch;";
        if (fail) throw new RuntimeException("delegate");
        if (choose) this.other = 11;
        else this.other = 13;
        try {
            if (choose) throw new IllegalArgumentException("caught");
            FinalDelegationTrace.text += "normal;";
        } catch (IllegalArgumentException failure) {
            FinalDelegationTrace.text += "caught;";
        }
    }
    ConstructorFinalDelegation(int value, boolean choose, boolean fail) {
        this(choose, fail);
        FinalDelegationTrace.text += "caller;";
        this.value = value;
    }
    int observed() { return value; }
    private static int constantZero() {
        FinalDelegationTrace.text += "guard;";
        return 0;
    }
    private static int argument(int index) {
        FinalDelegationTrace.text += "argument:" + index + ";";
        if (FinalDelegationTrace.fail == index) throw new RuntimeException();
        return index;
    }
    public static void main(String[] args) {
        for (int mode = 0; mode < 3; mode++) {
            FinalDelegationTrace.text = "";
            FinalDelegationTrace.fail = mode;
            try {
                ConstructorFinalDelegation result = new ConstructorFinalDelegation(42);
                System.out.println(result.value + ":" + result.other);
            }
            catch (RuntimeException failure) { System.out.println("failed"); }
            System.out.println(FinalDelegationTrace.text);
            FinalDelegationTrace.text = "";
            try {
                ConstructorFinalDelegation result = new ConstructorFinalDelegation(63, 0);
                System.out.println(result.value + ":" + result.other);
            }
            catch (RuntimeException failure) { System.out.println("failed"); }
            System.out.println(FinalDelegationTrace.text);
        }
        FinalDelegationTrace.text = "";
        System.out.println(new ConstructorFinalDelegation(false).value);
        System.out.println(FinalDelegationTrace.text);
        for (int mode = 0; mode < 2; mode++) {
            FinalDelegationTrace.text = "";
            System.out.println(new ConstructorFinalDelegation("conditional", mode == 0).conditional);
            System.out.println(FinalDelegationTrace.text);
        }
        FinalDelegationTrace.text = "";
        ConstructorFinalDelegation indexed = new ConstructorFinalDelegation(new int[] { 7 });
        System.out.println(indexed.value + ":" + indexed.other);
        System.out.println(FinalDelegationTrace.text);
        try { new ConstructorFinalDelegation(new int[0]); }
        catch (RuntimeException failure) { System.out.println(failure.getClass().getName()); }
        try { new ConstructorFinalDelegation((int[]) null); }
        catch (RuntimeException failure) { System.out.println(failure.getClass().getName()); }
        for (int mode = 0; mode < 3; mode++) {
            FinalDelegationTrace.text = "";
            try {
                ConstructorFinalDelegation result = new ConstructorFinalDelegation(23, mode == 0, mode == 2);
                System.out.println(result.value + ":" + result.other);
            } catch (RuntimeException failure) { System.out.println("failed"); }
            System.out.println(FinalDelegationTrace.text);
        }
    }
}
