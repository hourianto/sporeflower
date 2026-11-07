interface DispatchContract {
    int missing(int value);
    int present();
}

abstract class DispatchBase {
    protected abstract int gap();
    public int narrowed() { return 1; }
}

class DispatchImpl extends DispatchBase implements DispatchContract {
    public int missing(int value) { return value; }
    protected int gap() { return 7; }
    public int narrowed() { return 9; }
    public int present() { return 8; }
}

class DispatchLeaf extends DispatchImpl {}

class ConstructorParent {
    static int calls;
    ConstructorParent(int first, String second) throws java.io.IOException { calls++; }
}

class Constructorless extends ConstructorParent {
    Constructorless() throws java.io.IOException { super(3, "unused"); }
}

class ConstructorlessChild extends Constructorless {
    ConstructorlessChild() throws java.io.IOException { super(); }
}

public class DispatchContracts {
    public static void main(String[] args) {
        DispatchImpl value = new DispatchLeaf();
        System.out.println(value.present());
        System.out.println(((DispatchBase)value).narrowed());
        try { ((DispatchContract)value).missing(4); }
        catch (AbstractMethodError expected) { System.out.println(true); }
        try { ((DispatchBase)value).gap(); }
        catch (AbstractMethodError expected) { System.out.println(true); }
        try { Constructorless.class.newInstance(); }
        catch (InstantiationException expected) { System.out.println(true); }
        catch (IllegalAccessException unexpected) { throw new Error(); }
        try { ConstructorlessChild.class.newInstance(); }
        catch (InstantiationException expected) { System.out.println(true); }
        catch (IllegalAccessException unexpected) { throw new Error(); }
        System.out.println(ConstructorParent.calls);
    }
}
