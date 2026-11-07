public class FieldArrayUpdates {
    int i;
    long l;
    float f;
    double d;
    byte b;
    short s;
    char c;
    boolean flag;
    volatile int observed;
    static int effects;
    static int selected;
    static FieldArrayUpdates first;
    static FieldArrayUpdates second;
    static int[] firstArray;
    static int[] secondArray;

    static FieldArrayUpdates receiver() {
        effects = effects * 10 + 1;
        return selected++ == 0 ? first : second;
    }
    static int[] array() {
        effects = effects * 10 + 2;
        return selected++ == 0 ? firstArray : secondArray;
    }
    static int index() { effects = effects * 10 + 3; return selected++ & 1; }
    static int right() { effects = effects * 10 + 4; return 3; }
    static int mutate(int[] value) { effects = effects * 10 + 5; value[0] = 100; return 3; }
    int mutateField() { effects = effects * 10 + 6; i = 100; return 3; }
    int mutateVolatile() { effects = effects * 10 + 7; observed = 100; return 3; }
    boolean mutateFlag() { flag = false; return true; }

    void ordinary(int amount, long larger, float factor, double divisor) {
        i = i + amount;
        l = l - larger;
        f = f * factor;
        d = d / divisor;
        b = (byte)(b | amount);
        s = (short)(s << amount);
        c = (char)(c ^ amount);
        flag = flag ^ mutateFlag();
    }
    void promoted() {
        i = (int)((float)i + 1.0F);
        l = (long)((double)l + 1.0D);
        b = (byte)(b * 0.5D);
        s = (short)(s + 65536L);
        c = (char)(c + 2);
        f = (float)((double)f + 1.0D);
    }
    void changedRead() { i = (byte)i + 7; }
    void wideShift() { i = (int)((long)i >>> 40); }
    void fieldMutation() { i = i + mutateField(); }
    void volatileMutation() { observed = observed + mutateVolatile(); }
    void savedField() { int old = i; mutateField(); i = old + 3; }
    static void separateReceivers() { receiver().i = receiver().i + right(); }
    static void capturedReceiver() { receiver().i += right(); }
    static void separateArrays() { array()[0] = array()[0] + right(); }
    static void capturedArray() { array()[index()] += right(); }
    static void separateIndexes(int[] value) { value[index()] = value[index()] + right(); }
    static void arrayMutation(int[] value) { value[0] = value[0] + mutate(value); }
    static void savedArray(int[] value) { int old = value[0]; mutate(value); value[0] = old + 3; }
    static void byteArray(byte[] value, int mask) { value[1] = (byte)(value[1] | mask); }
    static void longArray(long[] value, long amount) { value[0] = value[0] + amount; }
    static void arrayFailure(int[] value) { value[index()] = value[index()] + right(); }
    static void capturedArrayFailure(int[] value) { value[index()] += right(); }
    static void differentIndex(int[] value) { value[1] = value[0] + right(); }
    static void nullField(FieldArrayUpdates value) { value.i = value.i + right(); }

    static void reset() {
        effects = 0;
        selected = 0;
        first.i = 10;
        second.i = 20;
        firstArray[0] = 10;
        firstArray[1] = 11;
        secondArray[0] = 20;
        secondArray[1] = 21;
    }
    static void state() {
        System.out.println(effects);
        System.out.println(first.i);
        System.out.println(second.i);
        System.out.println(firstArray[0]);
        System.out.println(firstArray[1]);
        System.out.println(secondArray[0]);
    }

    public static void main(String[] args) {
        first = new FieldArrayUpdates();
        second = new FieldArrayUpdates();
        firstArray = new int[2];
        secondArray = new int[2];
        first.i = 5; first.l = 20L; first.f = -0.0F; first.d = 21.0D;
        first.b = 64; first.s = 16000; first.c = 65530; first.flag = true;
        first.ordinary(3, 4L, -2.0F, 3.0D);
        System.out.println(first.i); System.out.println(first.l);
        System.out.println(Float.floatToIntBits(first.f)); System.out.println(first.d);
        System.out.println(first.b); System.out.println(first.s); System.out.println((int)first.c); System.out.println(first.flag);
        first.i = 2147483647; first.l = 9223372036854775807L; first.b = 127; first.s = 32760; first.c = 65535; first.f = 16777216.0F;
        first.promoted();
        System.out.println(first.i); System.out.println(first.l); System.out.println(first.b);
        System.out.println(first.s); System.out.println((int)first.c); System.out.println(first.f);
        first.i = 130; first.changedRead(); System.out.println(first.i);
        first.i = 65536; first.wideShift(); System.out.println(first.i);
        reset(); first.fieldMutation(); state();
        reset(); first.savedField(); state();
        reset(); first.observed = 10; first.volatileMutation(); System.out.println(first.observed); System.out.println(effects);
        reset(); separateReceivers(); state();
        reset(); capturedReceiver(); state();
        reset(); separateArrays(); state();
        reset(); capturedArray(); state();
        reset(); separateIndexes(firstArray); state();
        reset(); arrayMutation(firstArray); state();
        reset(); savedArray(firstArray); state();
        byte[] bytes = new byte[] { 0, 64 }; byteArray(bytes, 135); System.out.println(bytes[1]);
        long[] longs = new long[] { 9223372036854775807L }; longArray(longs, 2L); System.out.println(longs[0]);
        reset(); try { arrayFailure(null); } catch (NullPointerException expected) { System.out.println(effects); }
        reset(); try { arrayFailure(new int[1]); } catch (ArrayIndexOutOfBoundsException expected) { System.out.println(effects); }
        reset(); try { capturedArrayFailure(null); } catch (NullPointerException expected) { System.out.println(effects); }
        reset(); try { capturedArrayFailure(new int[0]); } catch (ArrayIndexOutOfBoundsException expected) { System.out.println(effects); }
        reset(); try { differentIndex(new int[1]); } catch (ArrayIndexOutOfBoundsException expected) { System.out.println(effects); }
        reset(); try { nullField(null); } catch (NullPointerException expected) { System.out.println(effects); }
    }
}
