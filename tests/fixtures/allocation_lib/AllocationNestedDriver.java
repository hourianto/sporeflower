public class AllocationNestedDriver {
    public static void main(String[] args) throws Exception {
        int mode = Integer.parseInt(args[0]);
        AllocationState.mode = mode;
        System.out.println(AllocationNestedClient.run(mode));
        System.out.println(AllocationState.trace);
    }
}
