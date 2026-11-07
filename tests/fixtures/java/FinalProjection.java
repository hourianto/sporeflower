class FinalProjectionBase {
    FinalProjectionBase() { FinalProjection.note(observe()); }
    int observe() { return -1; }
}
class FinalMetadata extends FinalProjectionBase {
    static final int ATTRIBUTE_VALUE = 4;
    int value;
    FinalMetadata() { FinalProjection.note(value); value = 4; FinalProjection.note(value); }
    FinalMetadata(boolean ignored) { FinalProjection.note(value); }
    FinalMetadata(int mode) {
        FinalProjection.note(value);
        if (mode != 0) value = mode;
        FinalProjection.note(value);
    }
    int observe() { return value; }
}
class FinalUpdate {
    int value;
    FinalUpdate() { value++; FinalProjection.note(value); }
    FinalUpdate(boolean ignored) { FinalProjection.note(value++); FinalProjection.note(value); }
    FinalUpdate(int ignored) { FinalProjection.note(++value); FinalProjection.note(value); }
    FinalUpdate(String ignored) { FinalProjection.note(value--); FinalProjection.note(value); }
    FinalUpdate(long ignored) { FinalProjection.note(--value); FinalProjection.note(value); }
}
class FinalOther {
    int value;
    FinalOther() { value = 5; }
    FinalOther(FinalOther other) { other.value = 7; FinalProjection.note(value); }
}
public class FinalProjection {
    static String trace = "";
    static void note(int value) { trace += value + ":"; }
    public static void main(String[] args) {
        if (args.length != 0) {
            FinalOther before = new FinalOther();
            FinalOther after = new FinalOther(before);
            System.out.println(before.value + ":" + after.value + ":" + trace);
            return;
        }
        System.out.println(new FinalMetadata().value);
        System.out.println(new FinalMetadata(false).value);
        System.out.println(new FinalMetadata(0).value);
        System.out.println(new FinalMetadata(8).value);
        System.out.println(trace);
        trace = "";
        System.out.println(new FinalUpdate().value);
        System.out.println(new FinalUpdate(false).value);
        System.out.println(new FinalUpdate(0).value);
        System.out.println(new FinalUpdate("").value);
        System.out.println(new FinalUpdate(0L).value);
        System.out.println(trace);
    }
}
