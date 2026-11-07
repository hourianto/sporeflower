package client;
import java.io.InputStream;
import orig.ResourceNames;

public class ResourceDriver {
    static int read(InputStream stream) throws Exception {
        if (stream == null) return -1;
        int value = stream.read();
        stream.close();
        return value;
    }
    static InputStream invoke(int kind, boolean nil, boolean flag) {
        switch (kind) {
            case 0: return ResourceNames.absolute(nil);
            case 1: return ResourceNames.concatenated(nil, flag);
            case 2: return ResourceNames.chosen(nil, flag);
            case 3: return ResourceNames.nullName(nil);
            case 4: return ResourceNames.relative(nil);
            case 5: return ResourceNames.dynamic(nil, flag ? "/token.txt" : "token.txt");
            case 6: return ResourceNames.nullablePrefix(nil, flag);
            case 7: return ResourceNames.characterPrefix(nil, flag);
            default: return ResourceNames.mutableName(nil);
        }
    }
    public static void main(String[] args) throws Exception {
        for (int kind = 0; kind < 9; kind++) {
            for (int mode = 0; mode < 4; mode++) {
                ResourceNames.trace = "";
                try { System.out.println(read(invoke(kind, (mode & 1) != 0, (mode & 2) != 0))); }
                catch (Throwable failure) { System.out.println(failure.getClass().getName()); }
                System.out.println(ResourceNames.trace);
            }
        }
    }
}
