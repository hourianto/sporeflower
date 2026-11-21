/** Compiled with the legacy javac using -XDjsrlimit=0 -source 1.3 -target 1.1. */
public class LegacyLookup {
    static void fromFinally(boolean lookup) throws Exception {
        String name = null;
        ordinary: {
            reflection: {
                try {
                    if (lookup) break reflection;
                    break ordinary;
                } finally {
                    name = "LegacyLookup";
                }
            }
            System.out.println(Class.forName(name).getName());
            return;
        }
        System.out.println(name);
    }

    static void lookupInFinally(boolean early) throws Exception {
        String shared = "LegacyLookup";
        System.out.println(shared);
        try {
            if (early) return;
        } finally {
            System.out.println(Class.forName("LegacyLookup").getName());
            System.out.println(Class.forName(shared).getName());
        }
    }

    public static void main(String[] args) throws Exception {
        fromFinally(false);
        fromFinally(true);
        lookupInFinally(false);
        lookupInFinally(true);
    }
}
