class FinalConstantReset {
    static final int ATTRIBUTE_VALUE = 4;
    static int value = 4;
    static byte read(boolean choose, byte small) { return (byte) (choose ? small : value); }
    static void reset(int next) { value = next; }
}
class FinalConstantInit {
    static final int ATTRIBUTE_VALUE = 4;
    static int value;
    static byte read(boolean choose, byte small) { return (byte) (choose ? small : value); }
    static { value = 129; }
}
class FinalConstantStable {
    static final int ATTRIBUTE_VALUE = 4;
    static int value;
    static byte read(boolean choose, byte small) { return (byte) (choose ? small : value); }
}
class FinalConstantConsumer {
    static byte read(boolean choose, byte small) {
        return (byte) (choose ? small : FinalConstantInit.value);
    }
    static String stable() { return new StringBuffer().append("").append(FinalConstantStable.value).toString(); }
    static String mutable() { return new StringBuffer().append("").append(FinalConstantInit.value).toString(); }
}
class FinalConstantResetOrder {
    static final int ATTRIBUTE_VALUE = 4;
    static int value;
    static { reset(7); value = 4; }
    static void reset(int next) { value = next; }
}
public class FinalConstants {
    public static void main(String[] args) {
        System.out.println(FinalConstantReset.read(false, (byte) 5));
        FinalConstantReset.reset(257);
        System.out.println(FinalConstantReset.read(false, (byte) 5));
        System.out.println(FinalConstantInit.read(false, (byte) 5));
        System.out.println(FinalConstantConsumer.read(false, (byte) 5));
        System.out.println(FinalConstantInit.read(true, (byte) 5));
        System.out.println(FinalConstantStable.read(false, (byte) 5));
        System.out.println(FinalConstantConsumer.stable() == "4");
        System.out.println(FinalConstantConsumer.mutable() == "129");
        System.out.println(FinalConstantResetOrder.value);
    }
}
