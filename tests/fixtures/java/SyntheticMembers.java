public class SyntheticMembers {
    private int this$0;
    private int val$arg;
    private int class$;
    public int this$value() { return this$0 + val$arg + class$; }
    public static void main(String[] args) {
        SyntheticMembers value = new SyntheticMembers();
        value.this$0 = 7;
        value.val$arg = 5;
        value.class$ = 3;
        System.out.println(value.this$value());
    }
}
