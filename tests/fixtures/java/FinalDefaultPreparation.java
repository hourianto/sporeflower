class FinalZeroBase {
    static int seen = observe();
    static int observe() {
        FinalZeroChild.reset(7);
        return FinalZeroChild.value;
    }
}
class FinalZeroChild extends FinalZeroBase {
    static final int ATTRIBUTE_ZERO = 0;
    static int value;
    static int after;
    static { after = value; }
    static void reset(int next) { value = next; }
}
class FinalStableBase {
    static int seen = FinalStableChild.value;
}
class FinalStableChild extends FinalStableBase {
    static final int ATTRIBUTE_VALUE = 4;
    static int value;
}
public class FinalDefaultPreparation {
    public static void main(String[] args) {
        new FinalZeroChild();
        System.out.println(FinalZeroBase.seen + ":" + FinalZeroChild.after + ":" + FinalZeroChild.value);
        new FinalStableChild();
        System.out.println(FinalStableBase.seen + ":" + FinalStableChild.value);
    }
}
