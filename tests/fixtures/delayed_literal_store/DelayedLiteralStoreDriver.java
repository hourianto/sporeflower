public class DelayedLiteralStoreDriver {
    static int trace;

    public static void step(int flags, int bit) {
        trace = trace * 31 + bit;
        if ((flags & bit) != 0) throw new RuntimeException();
    }

    public static void main(String[] args) {
        for (int flags = 0; flags < 16; flags++) {
            trace = 0;
            int result = DelayedLiteralStore.run(flags);
            int expected = (flags & 1) != 0 ? flags + 33 : (flags & 4) != 0 ? 81 : 49;
            if (result != expected || trace != 1) throw new RuntimeException("wrong store boundary");
            System.out.println(flags + ":" + result + ":" + trace);
        }
    }
}
