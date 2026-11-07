package typescope;

import typesupport.Widget;
import typesupport.Token;

public class Contexts extends Parent {
    static int Widget = 103;
    static int String = 107;
    Widget current;

    public static Widget create() {
        return new Widget(Trace.value == 0 ? Trace.record(2) : Trace.record(3));
    }
    public static Widget cast(Object Widget) { return (Widget) Widget; }
    public static Token inherited(Object Token) { return (Token) Token; }
    public static Widget[] array(Widget Widget) { return new Widget[] { Widget }; }
    public static boolean matches(Object value) { return value instanceof Widget; }
    public static Class literal() { return Widget.class; }
    public static String copy(String String) { return new String(String); }
    public static String text(Object String) { return java.lang.String.valueOf(String); }

    public static int staticValues() {
        return typesupport.Widget.total + typesupport.Widget.read()
            + typesupport.Token.number + typesupport.Token.read();
    }

    public static int fields() { return Widget + Token + String; }

    public static int packageNames(typesupport.prefix value, prefix.left.Item left, prefix.right.Item right) {
        return typesupport.prefix.number + left.value() + right.value();
    }
}
