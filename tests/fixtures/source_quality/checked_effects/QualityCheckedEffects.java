import java.io.IOException;

public final class QualityCheckedEffects {
    private static int trace;

    private static int mark(int value) {
        trace = trace * 31 + value;
        return value;
    }

    private static int checked(boolean fail) throws IOException {
        mark(3);
        if (fail) throw new IOException();
        return mark(260);
    }

    public static int zeroTrips() {
        try {
            mark(1);
            for (int unused = 2; unused < 1; unused++) checked(true);
            return 7;
        } catch (IOException failure) {
            return mark(9);
        }
    }

    public static int retainedCall(boolean fail) {
        try {
            int result = checked(fail);
            return (byte) result;
        } catch (IOException failure) {
            return mark(10);
        }
    }

    public static Object retainedConstructor(boolean fail) {
        try {
            CheckedBox result = new CheckedBox(fail);
            return result;
        } catch (IOException failure) {
            mark(11);
            return null;
        }
    }

    public static void main(String[] args) {
        System.out.println(zeroTrips());
        System.out.println(retainedCall(false));
        System.out.println(retainedCall(true));
        System.out.println(retainedConstructor(false) != null);
        System.out.println(retainedConstructor(true) != null);
        System.out.println(trace);
    }
}

final class CheckedBox {
    CheckedBox(boolean fail) throws IOException {
        if (fail) throw new IOException();
    }
}
