public class ReferenceOperations {
    static int effects;
    static int index() { effects = effects * 10 + 1; return 0; }
    static int value() { effects = effects * 10 + 2; return 7; }

    static int nullStore() {
        int[] array = null;
        try { array[index()] = value(); } catch (NullPointerException expected) { return effects; }
        return -1;
    }

    static int nullLength() {
        long[] array = null;
        try { return array.length; } catch (NullPointerException expected) { return 9; }
    }

    static int freshArrays() {
        effects = 0;
        (new int[2])[index()] = value();
        (new byte[2])[index()]++;
        return effects + (new int[] { value(), 8 })[index()];
    }

    static int nullThrow() {
        try { throw (RuntimeException)null; }
        catch (NullPointerException expected) { return 11; }
    }

    static String referenceStore(Object value, int index, boolean nullArray) {
        Object[] array = nullArray ? null : new String[1];
        try { array[index] = value; return "ok"; }
        catch (Throwable failure) { return failure.getClass().getName(); }
    }

    static String referenceInitializer(Object value) {
        try {
            Object[] array = new String[2];
            array[1] = value;
            return "ok:" + array[1];
        } catch (Throwable failure) { return failure.getClass().getName(); }
    }

    static boolean equal(ReferenceLeft left, ReferenceRight right) { return (Object) left == right; }
    static boolean instance(ReferenceLeft left) { return (Object) left instanceof ReferenceRight; }
    static ReferenceRight cast(ReferenceLeft left) { return (ReferenceRight) (Object) left; }

    public static void main(String[] args) {
        System.out.println(nullStore());
        System.out.println(nullLength());
        System.out.println(freshArrays());
        System.out.println(nullThrow());
        System.out.println(equal(null, null));
        System.out.println(equal(new ReferenceLeft(), new ReferenceRight()));
        System.out.println(instance(new ReferenceLeft()));
        System.out.println(cast(null));
        try { cast(new ReferenceLeft()); } catch (ClassCastException expected) { System.out.println("cast"); }
        System.out.println(referenceStore(new Object(), 0, false));
        System.out.println(referenceStore(new Object(), 2, false));
        System.out.println(referenceStore(new Object(), 0, true));
        System.out.println(referenceStore("x", 0, false));
        System.out.println(referenceStore(null, 0, false));
        System.out.println(referenceInitializer(new Object()));
        System.out.println(referenceInitializer("x"));
        System.out.println(referenceInitializer(null));
    }
}
final class ReferenceLeft {}
final class ReferenceRight {}
