public class FinallyContinuation {
    public static int closes;

    private static Object open(int mode) throws java.io.IOException {
        if ((mode & 3) == 1) throw new java.io.IOException();
        return new Object();
    }
    private static void work(int mode) throws java.io.IOException {
        if ((mode & 3) == 2) throw new java.io.IOException();
    }
    private static void log(int mode) {
        if ((mode & 4) != 0) throw new IllegalStateException();
    }
    private static void close(Object resource) { closes++; }

    public static void run(int mode) {
        Object resource = null;
        try { resource = open(mode); work(mode); }
        catch (Exception saved) { log(mode); }
        finally { try { close(resource); } catch (Exception ignored) {} }
    }
}
