import java.io.DataInputStream;
import java.io.IOException;
import java.io.InputStream;

public class CleanupFragments implements Runnable {
    public static Class type;
    public static Throwable failure;
    public static Throwable closeFailure;
    public static int closes;
    public static byte[] data;

    public static class Payload {
        public Payload() throws Throwable {
            if (failure != null) throw failure;
        }
    }

    public static class Stream extends InputStream {
        public int read() { return -1; }
        public void close() throws IOException {
            closes++;
            if (closeFailure instanceof IOException) throw (IOException) closeFailure;
            if (closeFailure != null) throw (RuntimeException) closeFailure;
        }
    }

    private static void read(DataInputStream stream) throws IOException {
        stream.read();
        try { type.newInstance(); }
        catch (InstantiationException failure) { throw new Error(); }
        catch (IllegalAccessException failure) { throw new Error(); }
    }

    public static void load() {
        byte[] snapshot = data;
        if (snapshot != null && snapshot.length > 0) {
            DataInputStream stream = new DataInputStream(new Stream());
            try { read(stream); }
            catch (IOException ignored) { return; }
            finally {
                try { stream.close(); }
                catch (IOException ignored) {}
            }
        }
    }

    public void run() { load(); }
}
