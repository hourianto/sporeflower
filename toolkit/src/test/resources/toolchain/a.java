public class a {
    public a(int direction) { if (direction == 1) a[0]++; }
    public a() { this(1); }
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

    public static boolean f(int input) { return b(input) == 1; }
    public static boolean g(int input) { return b(input) == 1; }
    public static int[] r = {1, 2, 0, 1};
    public static boolean h(int index) { return r[index * 2] == 1; }
}
