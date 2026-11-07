class LinkageState {
    static String trace = "";
    static int mode;
}
class LinkageTarget {
    static {
        LinkageState.trace += "target-init;";
        if (LinkageState.mode == 1) throw new RuntimeException("target init");
    }
    static int oldStatic(int value) { return value; }
    int oldVirtual(int value) { return value; }
}
class LinkageParent {
    protected int oldSpecial(int value) { return value; }
}
class LinkageBroad_ {}
class LinkageNarrow extends LinkageBroad_ {}
class LinkageCtorParent {
    LinkageCtorParent() { LinkageState.trace += "parent-construct;"; }
}
class LinkageCtor extends LinkageCtorParent {
    static {
        LinkageState.trace += "ctor-init;";
        if (LinkageState.mode == 1) throw new RuntimeException("ctor init");
    }
    LinkageCtor(LinkageNarrow first, int second) { LinkageState.trace += "construct;"; }
}
class LinkageCaller extends LinkageParent {
    private static int argument() {
        LinkageState.trace += "argument;";
        if (LinkageState.mode == 2) throw new IllegalArgumentException("argument");
        return 7;
    }
    private static LinkageTarget receiver() {
        LinkageState.trace += "receiver;";
        return null;
    }
    private static LinkageNarrow first() {
        LinkageState.trace += "first;";
        return null;
    }
    static int staticCall() { return LinkageTarget.oldStatic(argument()); }
    static int virtualCall() { return receiver().oldVirtual(argument()); }
    int specialCall() { return super.oldSpecial(argument()); }
    static LinkageCtor constructorCall() { return new LinkageCtor(first(), argument()); }
}
public class LinkageProbe {
    public static void main(String[] args) {
        LinkageState.mode = Integer.parseInt(args[1]);
        try {
            if (args[0].equals("static")) System.out.println(LinkageCaller.staticCall());
            else if (args[0].equals("virtual")) System.out.println(LinkageCaller.virtualCall());
            else if (args[0].equals("special")) System.out.println(new LinkageCaller().specialCall());
            else if (args[0].equals("reflect")) LinkageCtor.class.newInstance();
            else System.out.println(LinkageCaller.constructorCall());
        } catch (Throwable failure) {
            System.out.println(failure.getClass().getName());
        }
        System.out.println(LinkageState.trace);
    }
}
