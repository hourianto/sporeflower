import java.lang.reflect.Field;
import sun.misc.Unsafe;

// Compiled against the unpatched fixture, then its call descriptors are patched
// too. This caller can supply arbitrary JVM ints to a Z parameter. Unsafe also
// exercises the raw storage ordinary Z writes can retain on phoneME CLDC;
// modern JVM stores normalize those bits, so this driver injects them directly.
public class BooleanProjectionDriver {
    public static void main(String[] args) throws Exception {
        Field access = Unsafe.class.getDeclaredField("theUnsafe");
        access.setAccessible(true);
        Unsafe memory = (Unsafe) access.get(null);
        Field field = BooleanProjection.class.getDeclaredField("raw");
        Object base = memory.staticFieldBase(field);
        long offset = memory.staticFieldOffset(field);
        byte[] array = new byte[1];
        long element = memory.arrayBaseOffset(array.getClass());
        int[] values = { 0, 1, -1, -2, 2, 3, -128, 127 };
        for (int i = 0; i < values.length; i++) {
            int raw = values[i];
            memory.putByte(base, offset, (byte) raw);
            memory.putByte(array, element, (byte) raw);
            for (int mode = -2; mode < 5; mode++) {
                System.out.println(BooleanProjection.parameter(raw, mode));
                System.out.println(BooleanProjection.opposite(raw, mode));
                System.out.println(BooleanProjection.normalize(raw, mode));
                System.out.println(BooleanProjection.normalizeChoice(raw, mode));
                System.out.println(BooleanProjection.normalizeSelected(raw, mode));
                System.out.println(BooleanProjection.rawAlternative(raw, mode));
                System.out.println(BooleanProjection.canonical(raw, mode));
                System.out.println(BooleanProjection.local(raw, mode));
                BooleanProjection.trace = "";
                System.out.println(BooleanProjection.embedded(raw, mode));
                System.out.println(BooleanProjection.trace);
                System.out.println(BooleanProjection.loop(raw, mode));
                System.out.println(BooleanProjection.canonicalLoop(raw, mode));
                System.out.println(BooleanProjection.nested(raw, mode));
                System.out.println(BooleanProjection.field(mode));
                System.out.println(BooleanProjection.normalizeField(mode));
                System.out.println(BooleanProjection.array(array, mode));
                System.out.println(BooleanProjection.normalizeArray(array, mode));
                BooleanProjection.trace = "";
                System.out.println(BooleanProjection.logicalAndProjection(raw, mode));
                System.out.println(BooleanProjection.logicalOrProjection(raw, mode));
                System.out.println(BooleanProjection.trace);
                BooleanProjection.trace = "";
                new BooleanProjection(raw, mode);
                System.out.println(BooleanProjection.trace);
                BooleanProjection.trace = "";
                new BooleanProjectionChild(raw, mode);
                System.out.println(BooleanProjection.trace);
                System.out.println(BooleanProjection.precedence(raw, mode));
                BooleanProjection.trace = "";
                System.out.println(BooleanProjection.calls(raw, mode));
                System.out.println(BooleanProjection.trace);
                System.out.println(BooleanProjection.eagerParameter(raw, mode));
                System.out.println(BooleanProjection.eagerField(mode));
                BooleanProjection.trace = "";
                System.out.println(BooleanProjection.eagerArray(array, mode));
                System.out.println(BooleanProjection.trace);
                System.out.println(BooleanProjection.eagerXor(raw, mode));
                System.out.println(BooleanProjection.eagerLoop(raw, mode));
                BooleanProjection.trace = "";
                System.out.println(BooleanProjection.numericSink(raw, mode));
                System.out.println(BooleanProjection.trace);
                System.out.println(BooleanProjection.toggle(mode, raw));
                System.out.println(BooleanProjection.alternateToggle(mode, raw));
                BooleanProjection.trace = "";
                System.out.println(BooleanProjection.predicateCapture(raw, mode));
                System.out.println(BooleanProjection.predicateArrayCapture(array, mode));
                System.out.println(BooleanProjection.predicateCopyCapture(raw, mode));
                System.out.println(BooleanProjection.predicateCatchCapture(raw, mode));
                System.out.println(BooleanProjection.escapingCapture(raw, mode));
                System.out.println(BooleanProjection.exactOneCapture(raw, mode));
                System.out.println(BooleanProjection.maskCapture(raw, mode));
                System.out.println(BooleanProjection.catchExactOneCapture(raw, mode));
                System.out.println(BooleanProjection.predicateResultCapture(raw, mode));
                System.out.println(BooleanProjection.exactOneResultCapture(raw, mode));
                System.out.println(BooleanProjection.compoundAnd(raw ^ 3, mode));
                System.out.println(BooleanProjection.compoundOr(raw ^ 3, mode));
                System.out.println(BooleanProjection.compoundXor(raw ^ 3, mode));
                System.out.println(BooleanProjection.trace);
            }
        }
        BooleanProjection.trace = "";
        try {
            BooleanProjection.eagerArray(null, 1);
        } catch (NullPointerException expected) {
            System.out.println("null:" + BooleanProjection.trace);
        }
    }
}
