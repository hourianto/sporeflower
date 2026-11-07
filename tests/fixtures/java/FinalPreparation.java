class FinalPreparedBase {
    static int seen = FinalPreparedChild.value;
}
class FinalPreparedChild extends FinalPreparedBase {
    static final int ATTRIBUTE_VALUE = 4;
    static int value;
    static { value = 9; }
    static void reset(int next) { value = next; }
}
public class FinalPreparation {
    public static void main(String[] args) {
        new FinalPreparedChild();
        System.out.println(FinalPreparedBase.seen + ":" + FinalPreparedChild.value);
    }
}
