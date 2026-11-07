class InitializationPlacementState {
    static String trace = "";
    static boolean fail;
}
class InitializationPlacementTarget {
    static {
        InitializationPlacementState.trace += "init;";
        if (InitializationPlacementState.fail) throw new RuntimeException("initialization");
    }
    InitializationPlacementTarget(int value) {
        InitializationPlacementState.trace += "constructor:" + value + ";";
    }
}
public class InitializationPlacement {
    static void touch(int value) {
        InitializationPlacementState.trace += "touch:" + value + ";";
    }
    static void run(boolean choose) {
        int value = choose ? 7 : 8;
        touch(value);
        new InitializationPlacementTarget(value);
    }
    public static void main(String[] args) {
        InitializationPlacementState.fail = args[1].equals("fail");
        try { run(args[0].equals("left")); }
        catch (Throwable failure) { System.out.println(failure.getClass().getName()); }
        System.out.println(InitializationPlacementState.trace);
    }
}
