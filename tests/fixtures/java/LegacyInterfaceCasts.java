interface ObjectResult { Object value(); }
interface InheritedObjectResult extends ObjectResult {}
interface StringResult { String value(); }
interface IntResult { int value(); }
interface VoidResult { void value(); }
interface SameObjectResult { Object value(); }
interface OtherArguments { int value(int argument); }

class ObjectResultImpl implements InheritedObjectResult {
    public Object value() { return this; }
}

public class LegacyInterfaceCasts {
    static int trace;
    static InheritedObjectResult record(InheritedObjectResult value) {
        trace = trace * 10 + 1;
        return value;
    }
    static StringResult cast(InheritedObjectResult value) {
        return (StringResult) (Object) record(value);
    }
    static boolean test(InheritedObjectResult value) {
        return (Object) record(value) instanceof StringResult;
    }
    static boolean equal(InheritedObjectResult left, StringResult right) {
        return (Object) record(left) == right;
    }
    static VoidResult primitive(IntResult value) {
        return (VoidResult) (Object) value;
    }
    static StringResult[] array(InheritedObjectResult[] value) {
        return (StringResult[]) (Object) value;
    }
    static SameObjectResult compatible(InheritedObjectResult value) {
        return (SameObjectResult) value;
    }
    static OtherArguments overloaded(InheritedObjectResult value) {
        return (OtherArguments) value;
    }
    public static void main(String[] args) {
        InheritedObjectResult value = new ObjectResultImpl();
        System.out.println(cast(null) == null);
        try { cast(value); } catch (ClassCastException expected) {
            System.out.println(trace);
        }
        System.out.println(test(value));
        System.out.println(equal(null, null));
        System.out.println(primitive(null) == null);
        System.out.println(array(null) == null);
        System.out.println(trace);
    }
}
