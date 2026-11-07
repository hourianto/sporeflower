public class AllocationOrderProbe {
    static int state;
    // Legal in Java type contexts; it must not shadow a generated static
    // initialization trigger in an expression context.
    static boolean InitializationTarget;
    public static class Node {
        public int value;
        static { state = 10; }
        public Node(int value) { this.value = value; }
    }
    public static int run() {
        Node node = new Node(state++);
        return state * 100 + node.value;
    }
    public static void main(String[] args) {
        System.out.println(run());
        InitializationTarget target = new InitializationTarget(state++);
        System.out.println(state * 100 + target.value);
    }
}

class InitializationTarget {
    int value;
    static { AllocationOrderProbe.state = 20; }
    InitializationTarget(int value) { this.value = value; }
}
