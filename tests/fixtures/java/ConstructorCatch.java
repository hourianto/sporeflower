class ConstructorCatchBase {
    final int value;
    ConstructorCatchBase(int value) { this.value = value; }
}

public class ConstructorCatch extends ConstructorCatchBase {
    private static int effects;
    public String caught;
    public ConstructorCatch(int mode) {
        super(7);
        before(mode);
        try {
            if (mode < 0) throw new IllegalArgumentException("negative");
            caught = "normal";
        } catch (IllegalArgumentException failure) {
            caught = failure.getMessage();
        }
    }
    private static void before(int mode) { effects += mode; }
    public static void main(String[] args) {
        for (int mode : new int[] { -1, 2 }) {
            ConstructorCatch instance = new ConstructorCatch(mode);
            System.out.println(instance.value + ":" + instance.caught + ":" + effects);
        }
    }
}
