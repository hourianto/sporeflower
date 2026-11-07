import support.Base;
import support.Calls;
import support.Envelope;
import support.Face;
import support.Sub;
import support.IndexedEnvelope;

public class AllocationExpressions {
    public static Envelope create() {
        return new Envelope(Calls.title(), new Base[] { Calls.child(), null },
                new Face[] { Calls.child() }, new Base[][] { new Sub[] { Calls.child() } });
    }
    public static Object checkedStore(Object value) {
        Object[] values = new Sub[1];
        values[0] = value;
        return values;
    }
    public static int[] indexedArray(int[] values) {
        Calls.mark("indexed-array");
        return values;
    }
    public static int indexedCreate(int[] values, int index) {
        return new IndexedEnvelope(indexedArray(values)[index++], values[index++], index).sum;
    }
    public static void main(String[] args) {
        Calls.mode = Integer.parseInt(args[0]);
        try { create(); } catch (Throwable failure) { Calls.mark(failure.getClass().getName()); }
        try { Calls.mark("indexed-result" + indexedCreate(new int[] { 3, 5 }, 0)); }
        catch (Throwable failure) { Calls.mark(failure.getClass().getName()); }
        System.out.println(Calls.trace);
        try { checkedStore(new Object()); }
        catch (Throwable failure) { System.out.println(failure.getClass().getName()); }
        System.out.println(checkedStore(null) != null);
    }
}
