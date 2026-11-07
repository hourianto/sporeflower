package client;

public class ProtectedFields extends base.Parent {
    static int access(ProtectedFields value) {
        value.value = value.value + 3;
        return value.value;
    }
    static int hidden(Leaf value) { return ((ProtectedFields)value).value; }
    public static void main(String[] args) {
        Leaf value = new Leaf();
        System.out.println(access(value));
        System.out.println(hidden(value));
        System.out.println(value.value);
        System.out.println(value.grandValue());
    }
}

class Leaf extends ProtectedFields {
    public int value = 40;
}
