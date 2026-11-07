public class PhiProbe {
    public static int fib(int n) {
        int a = 0;
        int b = 1;
        while (n-- > 0) {
            int sum = a + b;
            a = b;
            b = sum;
        }
        return a;
    }
    public static int rotate(int n) {
        int a = 1;
        int b = 2;
        int c = 3;
        while (n-- > 0) {
            int save = a;
            a = b;
            b = c;
            c = save;
        }
        return a * 100 + b * 10 + c;
    }
    public static void main(String[] args) {
        for (int i = 0; i < 10; ++i) {
            System.out.println(fib(i));
            System.out.println(rotate(i));
        }
    }
}
