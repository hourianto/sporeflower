class ConstructorBase {
    final int base;
    ConstructorBase(int base) { this.base = base; }
}

public class ConstructorStraight extends ConstructorBase {
    private static int count;
    private final int value;
    public ConstructorStraight(int input) {
        super(input + 2);
        int saved = step();
        value = saved * saved;
    }
    private static int step() { return ++count; }
    public static void main(String[] args) {
        for (int i = 0; i < 5; i++) {
            ConstructorStraight value = new ConstructorStraight(i);
            System.out.println(value.base + ":" + value.value);
        }
    }
}
