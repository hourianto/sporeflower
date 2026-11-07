public class HelperRecovery {
    static int VALUE;
    static {
        int value;
        try {
            value = HelperRecoveryDriver.next();
        } catch (RuntimeException caught) {
            value = HelperRecoveryDriver.next();
        }
        VALUE = value;
    }
}
class HelperRecoveryDriver {
    static int next() { return 42; }
    public static void main(String[] args) { System.out.println(HelperRecovery.VALUE); }
}
