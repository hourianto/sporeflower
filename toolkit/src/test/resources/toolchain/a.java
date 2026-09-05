public class a {
    public static int[] a = new int[] { 10, 20 };

    public static int b(int value) {
        return value < 0 ? -value : value;
    }

    public static int c(int direction) {
        return b(direction) == 1 ? a[1] : a[0];
    }

    public static int d(int mask) {
        return mask & 3;
    }

    public static int e() {
        return d(3);
    }
}
