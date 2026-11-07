public class ConditionalPrecedence {
    static boolean compare(boolean flag, int n) { return flag == (n == 1); }
    static boolean bits(boolean a, boolean b, boolean c) { return (a ? b : c) ^ (b ? c : a); }
    static int nested(boolean a, boolean b, boolean c) { return ((a ? b : c) ? 7 : 13) + (a ? (b ? 1 : 2) : (c ? 3 : 4)); }
    public static void main(String[] args) {
        int result = 0;
        for (int i = 0; i < 8; i++) {
            boolean a = (i & 1) != 0;
            boolean b = (i & 2) != 0;
            boolean c = (i & 4) != 0;
            result = result * 31 + nested(a, b, c);
            result = result * 31 + (bits(a, b, c) ? 1 : 0);
            result = result * 31 + (compare(a, i - 2) ? 1 : 0);
        }
        System.out.println(result);
    }
}
