package orig;
import java.io.InputStream;

public class ResourceNames {
    public static String trace = "";
    public static Class receiver(boolean nil) {
        trace = trace + "receiver;";
        return nil ? null : ResourceNames.class;
    }
    public static String suffix(boolean fail) {
        trace = trace + "suffix;";
        if (fail) throw new IllegalStateException();
        return "token.txt";
    }
    public static InputStream absolute(boolean nil) {
        return receiver(nil).getResourceAsStream("/token.txt");
    }
    public static InputStream concatenated(boolean nil, boolean fail) {
        return receiver(nil).getResourceAsStream(new StringBuffer("/").append(suffix(fail)).toString());
    }
    public static InputStream characterPrefix(boolean nil, boolean fail) {
        return receiver(nil).getResourceAsStream(new StringBuffer().append('/').append(suffix(fail)).toString());
    }
    public static InputStream chosen(boolean nil, boolean root) {
        return receiver(nil).getResourceAsStream(root ? "/token.txt" : "/client/token.txt");
    }
    public static InputStream nullName(boolean nil) {
        return receiver(nil).getResourceAsStream(null);
    }
    public static InputStream relative(boolean nil) {
        return receiver(nil).getResourceAsStream("token.txt");
    }
    public static InputStream dynamic(boolean nil, String path) {
        return receiver(nil).getResourceAsStream(path);
    }
    public static InputStream nullablePrefix(boolean nil, boolean root) {
        return receiver(nil).getResourceAsStream(new StringBuffer().append(root ? "/" : null).append(suffix(false)).toString());
    }
    public static InputStream mutableName(boolean nil) {
        String path = "/token.txt";
        if (!nil) path = "token.txt";
        return receiver(nil).getResourceAsStream(path);
    }
}
