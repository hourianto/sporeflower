public class Binding extends BindingMiddle {
    public int value = 30;
    static String choose(Object value) { return "object"; }
    static String choose(String value) { return "string"; }
    static String choose(char[] value) { return "chars"; }
    static String choose(int value) { return "int"; }
    static String choose(byte value) { return "byte"; }
    static String choose(boolean value) { return value ? "true" : "false"; }
    static String choose(long value) { return "long"; }
    static String choose(float value) { return "float"; }
    static String choose(double value) { return "double"; }

    String overloads(byte value) {
        return choose((Object) "text") + choose((Object) null) + choose((int) value) + choose(value)
            + choose(true) + choose(value > 3 ? false : true) + choose((long) value)
            + choose((float) value) + choose((double) value);
    }

    int fields(Binding other) {
        return this.value + super.value + ((BindingBase) this).value + ((BindingBase) other).value;
    }

    public static void main(String[] args) {
        Binding object = new Binding();
        System.out.println(object.overloads((byte) 3));
        System.out.println(object.overloads((byte) 5));
        System.out.println(object.fields(object));
        System.out.println(BindingBase.privateThroughSubclass(object));
        System.out.println(BindingBase.privateFieldThroughSubclass(object));
        System.out.println(BindingBase.privateFieldThroughSubclass(object));
    }
}

class BindingBase {
    public int value = 10;
    private int secret = 41;
    private int privateValue() { return value; }
    static int privateThroughSubclass(Binding value) { return ((BindingBase)value).privateValue(); }
    static int privateFieldThroughSubclass(Binding value) {
        ((BindingBase)value).secret += 3;
        return ((BindingBase)value).secret;
    }
}

class BindingMiddle extends BindingBase {
    public int value = 20;
}
