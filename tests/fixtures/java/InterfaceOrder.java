public class InterfaceOrder {
    static int trace;

    static int record(int value) {
        trace = trace * 10 + value;
        return value;
    }

    static int observe() {
        return InitData.first * 100 + InitData.second * 10;
    }

    static boolean condition(int value) {
        trace = trace * 10 + value;
        return value != 0;
    }

    static int observeBranches() {
        return BranchData.first * 100 + BranchData.second * 10 + BranchData.observed;
    }

    public static void main(String[] args) {
        System.out.println(InitData.observed);
        System.out.println(trace);
        System.out.println(InitData.bytes[2]);
        System.out.println(InitData.rows[1][1]);
        System.out.println(BranchData.observed);
        System.out.println(trace);
    }
}

interface BranchData {
    int first = InterfaceOrder.condition(1) ? InterfaceOrder.record(3) : InterfaceOrder.record(4);
    int second = InterfaceOrder.condition(0) ? InterfaceOrder.record(5) : InterfaceOrder.observeBranches();
    int observed = InterfaceOrder.observeBranches();
}

interface InitData {
    int first = InterfaceOrder.record(1);
    int second = InterfaceOrder.record(first + 1);
    int observed = InterfaceOrder.observe();
    byte[] bytes = new byte[] {-128, 0, 127};
    int[][] rows = new int[][] {{1, 2}, {3, 4}};
}
