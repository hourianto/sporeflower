package typescope;

public class Driver {
    public static void main(String[] arguments) {
        typesupport.Widget item = Contexts.create();
        System.out.println(Trace.value);
        System.out.println(Contexts.cast(item).value);
        System.out.println(Contexts.inherited(new typesupport.Token(3)).value);
        System.out.println(Contexts.array(item)[0].value);
        System.out.println(Contexts.matches(item));
        System.out.println(Contexts.literal() == typesupport.Widget.class);
        System.out.println(Contexts.copy("copied"));
        System.out.println(Contexts.text("text"));
        System.out.println(Contexts.staticValues());
        System.out.println(Contexts.fields());
        System.out.println(Contexts.packageNames(new typesupport.prefix(), new prefix.left.Item(), new prefix.right.Item()));
    }
}
