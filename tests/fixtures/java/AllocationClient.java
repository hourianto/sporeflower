import java.io.IOException;

public class AllocationClient {
    private static void touch(Object receiver) {
        AllocationState.trace += "touch;";
        receiver.getClass();
    }
    private static int argument(int mode) throws IOException {
        AllocationState.trace += "argument;";
        if (mode == 2) throw new IOException("argument");
        return 7;
    }
    public static AllocationExternal run(Object receiver, int mode) throws IOException {
        // The test moves NEW/DUP before touch in the compiled code, preserving
        // every instruction and the original protected range boundaries.
        touch(receiver);
        return new AllocationExternal(argument(mode));
    }
    public static AllocationExternal guarded(Object receiver, int mode) {
        try {
            touch(receiver);
            return new AllocationExternal(argument(mode));
        } catch (IOException failure) {
            AllocationState.trace += "caught;";
            return null;
        }
    }
}
