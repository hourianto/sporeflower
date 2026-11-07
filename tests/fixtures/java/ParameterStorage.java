import java.io.ByteArrayInputStream;
import java.io.InputStream;

public class ParameterStorage {
    static String trace;

    static int step(int value, int fail) {
        trace += ":" + value;
        if (fail == 1) throw new IllegalArgumentException();
        return value + 1;
    }

    static int cases(int value, int choice) {
        switch (choice) {
            case 0: step(10, 0); break;
            case 1: step(20, 0); break;
            case 2: step(30, 0); break;
            case 3: value = step(40, 0); break;
            case 4: if (value > 0) step(50, 0); break;
            default: step(60, 0);
        }
        return value;
    }

    static int loop(int count, int value) {
        while (count-- > 0) value = step(value, 0);
        return value;
    }

    static int captured(int value, int choice) {
        int saved = value;
        if (choice > 0) value = step(value, 0);
        return saved * 1000 + value;
    }

    static int arguments(int left, int right, int choice) {
        if (choice == 0) left = right;
        else right = left;
        return left * 1000 + right;
    }

    static int commit(int value, int fail) {
        try {
            value = step(value, fail);
        } catch (IllegalArgumentException caught) {
            return value;
        }
        return value;
    }

    static int snapshot(int value, int fail) {
        int saved = value;
        try {
            value = step(value, fail);
            step(value, fail - 1);
        } catch (IllegalArgumentException caught) {
            return saved;
        }
        return value;
    }

    static InputStream reference(boolean choose, InputStream input) {
        InputStream result = null;
        if (choose) result = new ByteArrayInputStream(new byte[] { 23 });
        else if (input != null) result = input;
        return result;
    }

    public static void main(String[] args) throws Exception {
        for (int value = -3; value <= 3; value++) {
            for (int choice = 0; choice < 7; choice++) {
                trace = "";
                int a = cases(value, choice);
                int b = loop(choice, value);
                int c = captured(value, choice);
                int d = arguments(value, value + 5, choice);
                int e = commit(value, choice);
                int f = snapshot(value, choice);
                System.out.println(value + ":" + choice + ":" + a + ":" + b + ":" + c + ":" + d + ":" + e + ":" + f + trace);
            }
        }
        for (int mode = 0; mode < 4; mode++) {
            InputStream result = reference(mode < 2, mode % 2 == 0 ? null : new ByteArrayInputStream(new byte[] { 41 }));
            System.out.println(result == null ? -1 : result.read());
        }
    }
}
