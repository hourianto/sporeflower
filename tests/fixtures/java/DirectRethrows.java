public class DirectRethrows {
    static Throwable pending;
    static int log;

    static Object make(Class kind) {
        try { return kind.newInstance(); }
        catch (InstantiationException ex) { log += 1000; return null; }
        catch (IllegalAccessException ex) { log += 2000; return null; }
    }

    static Object declared(Class kind, Exception explicit) throws Exception {
        if (explicit != null) throw explicit;
        try { return make(kind); }
        catch (Exception ex) { log += 1; throw ex; }
    }

    static Object caught(Class kind) {
        try {
            try { return make(kind); }
            catch (Exception ex) { log += 2; throw ex; }
        } catch (Exception ex) { log += 4; return ex; }
    }

    static Object withFinally(Class kind) throws Exception {
        try {
            try { return make(kind); }
            catch (Exception ex) { log += 8; throw ex; }
        } finally { log += 16; }
    }

    static Object siblings(Class kind) throws Exception {
        try { return make(kind); }
        catch (RuntimeException ex) { log += 32; throw ex; }
        catch (Exception ex) { log += 64; throw ex; }
    }

    static Object fromCatch(Class kind) throws Exception {
        try { throw new java.io.IOException(); }
        catch (Exception outer) {
            try { return make(kind); }
            catch (Exception inner) { log += 128; throw inner; }
        }
    }

    static Object narrowCatch(Class kind) throws Exception {
        try {
            try { return make(kind); }
            catch (Exception ex) { log += 256; ex = ex; throw ex; }
        } catch (java.io.IOException ex) { log += 512; return ex; }
    }

    static Object call(int mode, Class kind) throws Exception {
        switch (mode) {
            case 0: return declared(kind, null);
            case 1: return caught(kind);
            case 2: return withFinally(kind);
            case 3: return siblings(kind);
            case 4: return fromCatch(kind);
            default: return narrowCatch(kind);
        }
    }

    public static void main(String[] args) throws Exception {
        Class kind = Class.forName("DirectPayload");
        Throwable[] values = {
            new Exception(), new java.io.IOException(), new RuntimeException(),
            new Error(), new Throwable(), new InstantiationException(),
            new IllegalAccessException()
        };
        for (int repeat = 0; repeat < 3; repeat++) {
            for (int mode = 0; mode < 6; mode++) {
                for (int index = 0; index < values.length; index++) {
                    pending = values[index];
                    log = 0;
                    Object result;
                    try { result = call(mode, kind); }
                    catch (Throwable value) { result = value; }
                    boolean filtered = index >= 5;
                    if (result != (filtered ? null : pending)) throw new Error("identity");
                    System.out.println(log);
                }
            }
        }
        Exception explicit = new Exception();
        try { declared(kind, explicit); throw new Error("returned"); }
        catch (Exception value) { if (value != explicit) throw new Error("explicit identity"); }
    }
}

class DirectPayload {
    public DirectPayload() throws Throwable { throw DirectRethrows.pending; }
}
