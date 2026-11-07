package pkg;
public class FinallyReturnState {
    public static Object stream;
    public static boolean connection, success, error;
    public static int failAt, cleanupFailAt;
    public static String trace;
    public static Object open() {
        step(1);
        return connection ? new Object() : null;
    }
    public static void step(int id) {
        trace += id;
        if (id == failAt || id == cleanupFailAt) {
            if (error) throw new AssertionError("step" + id);
            throw new IllegalStateException("step" + id);
        }
    }
    public static void caught(Exception exception) {
        trace += " caught:" + exception.getMessage();
    }
}
