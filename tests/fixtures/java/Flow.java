/** Behavioral oracle: compiled with -source 7 -target 7, without invokedynamic. */
public class Flow {
    private static int effects;
    private int field;

    public Flow(int n) { field = n; }

    public static int branches(int x, int y) {
        int a;
        if (x > y) a = x - y; else a = y - x;
        return (x < 0 && y != 0) || a == 7 ? a * 3 : a + 4;
    }

    public static int loops(int n) {
        int sum = 0;
        for (int i = 0; i < n; i++) {
            if (i == 3) continue;
            int j = 0;
            while (j < i) {
                if (j == 7) break;
                sum += i ^ j++;
            }
            if (sum > 10000) break;
        }
        do { sum--; } while (sum > 100);
        return sum;
    }

    public static int switches(int n) {
        int r = n;
        switch (n & 7) {
            case 0: r += 2;
            case 1: r *= 3; break;
            case 2: case 3: return -r;
            case 4: r = 17; break;
            default: r -= 19;
        }
        switch (n) {
            case -100: return r - 1;
            case 1000: return r + 1;
            default: return r;
        }
    }

    public static int arrays(int n) {
        byte[] b = new byte[] { -128, 1, 127 };
        char[] c = new char[] { 0, 32768, 65535 };
        boolean[] z = new boolean[] { true, false, n > 0 };
        int[][] m = new int[2][3];
        int index = n & 1;
        m[0][index] = b[index]++ + c[index];
        return m[0][index] + b[index] + (z[index] ? 100 : -100);
    }

    public static int catchOrder(int n) {
        try {
            if (n == 0) throw new IllegalArgumentException("zero");
            return 100 / n;
        } catch (IllegalArgumentException e) {
            return e.getMessage().length();
        } catch (RuntimeException e) {
            return -10;
        } finally {
            effects++;
        }
    }

    public int updates(int n) {
        int old = field++;
        return old + ++field + (field += n);
    }

    public static long arithmetic(long a, int b) {
        return ((a << b) ^ (a >>> b)) + (a / 3) - (a % 7);
    }

    public static int floating(float a, float b) {
        return (a < b ? 1 : 0) + (a >= b ? 2 : 0) + (a == b ? 4 : 0) + (a != b ? 8 : 0);
    }

    public static String text(int n, char ch, boolean flag) {
        return "n=" + n + ",ch=" + ch + ",flag=" + flag;
    }

    public static int typeJoin(boolean flag) {
        Object array = flag ? (Object) new byte[3] : (Object) new int[4];
        return array instanceof byte[] ? ((byte[]) array).length : ((int[]) array).length;
    }

    public static int synchronizedBlock(Object lock, int n) {
        synchronized (lock) {
            effects += n;
            if (n > 0) return effects;
        }
        return -1;
    }

    public static void main(String[] args) {
        long digest = 0;
        Flow object = new Flow(7);
        for (int i = -20; i < 30; i++) {
            digest = digest * 31 + branches(i, 9 - i);
            digest = digest * 31 + loops(i);
            digest = digest * 31 + switches(i);
            digest = digest * 31 + arrays(i);
            digest = digest * 31 + catchOrder(i);
            digest = digest * 31 + object.updates(i);
            digest = digest * 31 + arithmetic(i * 123456789L, i);
            digest = digest * 31 + typeJoin(i > 0);
            digest = digest * 31 + synchronizedBlock(object, i);
        }
        System.out.println(digest);
        System.out.println(effects);
        System.out.println(floating(Float.NaN, 1.0f));
        System.out.println(floating(1.0f, Float.NaN));
        System.out.println(floating(-0.0f, 0.0f));
        System.out.println(text(65535, '\uffff', true));
    }
}
