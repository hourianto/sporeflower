import java.lang.reflect.Field;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.Arrays;
import java.util.Hashtable;
import pkg.*;

public class ConstructorDriver {
    private static Object field(Object object, String name) throws Exception {
        Class type = object.getClass();
        while (type != null) {
            try { Field field = type.getDeclaredField(name); field.setAccessible(true); return field.get(object); }
            catch (NoSuchFieldException ignored) { type = type.getSuperclass(); }
        }
        throw new NoSuchFieldException(name);
    }
    public static void main(String[] args) throws Exception {
        for (int a : new int[] { -1, 0, 10 }) {
            for (int b : new int[] { 0, 20 }) {
                Hashtable values = new Hashtable();
                values.put("id", Integer.valueOf(a)); values.put("post_id", Integer.valueOf(30));
                values.put("from_id", Integer.valueOf(b)); values.put("owner_id", Integer.valueOf(40));
                Object instance = new TestConstructorPreinitMultiArg(values);
                System.out.println(field(instance, "left") + ":" + field(instance, "right"));
            }
        }
        for (String input : new String[] { "ok", "bad", null }) {
            try { System.out.println(new TestConstructorPreinitThrow(input).value()); }
            catch (RuntimeException failure) { System.out.println(failure.getClass().getName()); }
        }
        Hashtable values = new Hashtable(); values.put("from", "value"); values.put("other", "untouched");
        Object instance = new TestConstructorPreinitReusedArgument(values);
        System.out.println((field(instance, "map") == values) + ":" + values.get("to") + ":" + values.containsKey("from"));
        try { new TestConstructorPreinitReusedArgument(new Hashtable()); }
        catch (RuntimeException failure) { System.out.println(failure.getClass().getName()); }

        for (int fail = -1; fail <= 6; fail++) {
            TestConstructorPreludeEffects.events.clear(); TestConstructorPreludeEffects.failingStep = fail;
            try {
                TestConstructorOrderedPrelude ordered = new TestConstructorOrderedPrelude();
                System.out.println(ordered.first + ":" + ordered.second);
            } catch (RuntimeException failure) { System.out.println(failure.getMessage()); }
            System.out.println(TestConstructorPreludeEffects.events);
        }

        String first = new String("first"); StringBuffer second = new StringBuffer("second");
        TestConstructorPreludeBase.calls = 0;
        TestConstructorSavedArgumentsBeforeSuper saved = new TestConstructorSavedArgumentsBeforeSuper(first, second);
        System.out.println((saved == TestConstructorSavedArgumentsBeforeSuper.instance) + ":" + (first == TestConstructorSavedArgumentsBeforeSuper.first) + ":" + (second == TestConstructorSavedArgumentsBeforeSuper.second) + ":" + saved.ready);
        try { new TestConstructorSavedArgumentsBeforeSuper(new Object(), null); }
        catch (ClassCastException failure) { System.out.println("cast after super:" + TestConstructorPreludeBase.calls); }
        TestConstructorPreludeBase.failure = new IllegalStateException("super");
        TestConstructorSavedArgumentsBeforeSuper.instance = null;
        try { new TestConstructorSavedArgumentsBeforeSuper(first, second); }
        catch (RuntimeException failure) { System.out.println(failure.getMessage() + ":" + (TestConstructorSavedArgumentsBeforeSuper.instance == null)); }

        for (String payload : new String[] { "", "payload", "longer payload" }) {
            byte[] data = bytes(payload);
            TestConstructorPreludeInput input = new TestConstructorPreludeInput(data);
            TestConstructorStatefulThisPrelude parsed = new TestConstructorStatefulThisPrelude(input);
            System.out.println(parsed.id + ":" + parsed.label + ":" + parsed.payload + ":" + input.position);
            System.out.println(TestConstructorPreludeInput.events);
            if (payload.length() > 0) {
                input = new TestConstructorPreludeInput(Arrays.copyOf(data, data.length - 6));
                try { new TestConstructorStatefulThisPrelude(input); }
                catch (ArrayIndexOutOfBoundsException failure) { System.out.println("decode failed:" + TestConstructorPreludeInput.constructions + ":" + input.position); }
                System.out.println(TestConstructorPreludeInput.events);
            }
        }
    }
    private static byte[] bytes(String payload) {
        String label = "label";
        ByteBuffer buffer = ByteBuffer.allocate(16 + label.length() * 2 + payload.length()).order(ByteOrder.LITTLE_ENDIAN);
        buffer.putInt(17).putInt(label.length());
        for (int i = 0; i < label.length(); i++) buffer.putChar(label.charAt(i));
        buffer.putInt(payload.length());
        for (int i = 0; i < payload.length(); i++) buffer.put((byte) payload.charAt(i));
        buffer.putInt(123);
        return buffer.array();
    }
}
