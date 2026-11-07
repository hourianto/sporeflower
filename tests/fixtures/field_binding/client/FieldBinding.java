package client;

public class FieldBinding {
    public static void main(String[] args) {
        hidden.PublicChild child = new hidden.PublicChild();
        System.out.println(child.link != null);
        child.link = null;
        System.out.println(child.link == null);
        System.out.println(child.own());
    }
}
