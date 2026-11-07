public class AllocationDriver {
    public static void main(String[] args) {
        int mode = Integer.parseInt(args[0]);
        AllocationState.mode = mode;
        Object receiver = mode == 4 ? null : new Object();
        try {
            AllocationExternal value = args[1].equals("guarded")
                ? AllocationClient.guarded(receiver, mode)
                : AllocationClient.run(receiver, mode);
            System.out.println(value == null ? "null" : Integer.toString(value.value));
        } catch (Throwable failure) {
            System.out.println(failure.getClass().getName() + ":" + failure.getMessage());
        }
        System.out.println(AllocationState.trace);
    }
}
