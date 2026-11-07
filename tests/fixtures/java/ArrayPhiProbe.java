public class ArrayPhiProbe {
    public static boolean nullable(boolean choose) {
        boolean[] a = null;
        if (choose) a = new boolean[] {true};
        return a[0];
    }
    public static boolean choose(boolean select, boolean[] input) {
        boolean[] a;
        if (select) a = null;
        else a = input;
        return a[0];
    }
    public static int cyclic(int rounds) {
        byte[] data = null;
        int total = 0;
        for (int i = 0; i < rounds; i++) {
            try {
                byte[] next;
                if (data == null) next = new byte[1];
                else {
                    next = new byte[data.length + 1];
                    System.arraycopy(data, 0, next, 0, data.length);
                }
                data = next;
                if ((i & 1) == 0) throw new RuntimeException();
                total += data.length;
            } catch (RuntimeException expected) {
                total += data == null ? -100 : data.length;
            }
        }
        return total * 1000 + (data == null ? 0 : data.length);
    }
    public static void main(String[] args) {
        System.out.println(nullable(true));
        System.out.println(choose(false, new boolean[] {true}));
        for (int i = 0; i < 7; i++) System.out.println(cyclic(i));
    }
}
