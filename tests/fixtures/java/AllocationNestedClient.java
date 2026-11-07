import java.io.IOException;

public class AllocationNestedClient {
    private static int argument(int mode) throws IOException {
        AllocationState.trace += "argument;";
        if (mode == 3) throw new IOException("argument");
        return mode + 7;
    }
    public static int run(int mode) {
        try {
            return new AllocationOuter(new AllocationInner(5,
                (mode & 1) == 0 ? argument(mode) : argument(mode) + 1)).value;
        } catch (Throwable failure) {
            AllocationState.trace += failure.getClass().getName() + ";";
            return -1;
        }
    }
}
