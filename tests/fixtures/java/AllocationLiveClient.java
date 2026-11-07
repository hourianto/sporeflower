import java.io.IOException;

public class AllocationLiveClient {
    private static AllocationResource open(int mode) {
        AllocationState.trace += "open;";
        if (mode == 2) throw new IllegalStateException("open");
        return new AllocationResource(mode);
    }
    private static int marker(int mode) throws IOException {
        AllocationState.trace += "argument;";
        if (mode == 5) throw new IOException("argument");
        return 9;
    }
    private static void close(Object resource) {
        AllocationState.trace += resource == null ? "null;"
            : resource instanceof AllocationResource ? "closeResource;" : "closeWrapper;";
    }
    public static int run(int mode) {
        AllocationResource resource = null;
        AllocationWrapper wrapper = null;
        try {
            // The test moves Wrapper NEW/DUP before this assignment. The catch
            // must still see a resource opened before a later argument fails.
            resource = open(mode);
            wrapper = new AllocationWrapper(resource, marker(mode));
            AllocationState.trace += "use;";
            if (mode == 4) throw new IllegalArgumentException("use");
            int result = wrapper.value;
            close(resource);
            close(wrapper);
            return result;
        } catch (Throwable failure) {
            AllocationState.trace += failure.getClass().getName() + ";";
            close(resource);
            close(wrapper);
            return -1;
        }
    }
    public static int branch(int mode) {
        AllocationResource resource = null;
        AllocationWrapper wrapper = null;
        try {
            resource = open(mode);
            if (mode == 6) throw new IllegalArgumentException("before initialization");
            int value;
            if ((mode & 1) != 0) value = marker(mode);
            else value = marker(mode) + 1;
            wrapper = new AllocationWrapper(resource, value);
            close(resource);
            close(wrapper);
            return wrapper.value;
        } catch (Throwable failure) {
            AllocationState.trace += failure.getClass().getName() + ";";
            close(resource);
            close(wrapper);
            return -1;
        }
    }
    public static int loop(int mode) {
        AllocationResource resource = null;
        AllocationWrapper wrapper = null;
        try {
            resource = open(mode);
            int total = 0;
            int index = 0;
            while (index < 2) {
                total += marker(mode == 7 && index == 1 ? 5 : mode);
                index++;
            }
            wrapper = new AllocationWrapper(resource, total);
            close(resource);
            close(wrapper);
            return wrapper.value;
        } catch (Throwable failure) {
            AllocationState.trace += failure.getClass().getName() + ";";
            close(resource);
            close(wrapper);
            return -1;
        }
    }
    public static int owned(int mode) {
        AllocationResource resource = null;
        AllocationWrapper wrapper = null;
        try {
            resource = open(mode);
            int value;
            try {
                value = marker(mode);
            } catch (java.io.IOException failure) {
                AllocationState.trace += "inner;";
                resource = new AllocationResource(mode + 20);
                throw new IllegalStateException(failure.getMessage());
            }
            wrapper = new AllocationWrapper(resource, value);
            if (mode == 4) throw new IllegalArgumentException("after initialization");
            return wrapper.value;
        } catch (Throwable failure) {
            AllocationState.trace += failure.getClass().getName() + ";";
            if (resource != null) AllocationState.trace += "resource=" + resource.id + ";";
            close(resource);
            close(wrapper);
            return -1;
        }
    }
    public static int innerPlain(int mode) {
        AllocationResource resource = new AllocationResource(mode);
        try {
            int value;
            try {
                value = marker(mode);
            } catch (java.io.IOException failure) {
                AllocationState.trace += "plain-inner;";
                throw new IllegalStateException("plain failure");
            }
            return new AllocationWrapper(resource, value).value;
        } catch (Throwable failure) {
            AllocationState.trace += failure.getClass().getName() + ";";
            close(resource);
            close(null);
            return -1;
        }
    }
}
