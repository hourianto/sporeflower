package hidden;

public class PublicChild extends HiddenBase {
    private String mark = "child";
    public String own() { return mark; }
}

class HiddenBase {
    public Object link = new Object();
}
