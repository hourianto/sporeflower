public class AllocationLiveDriver {
    public static void main(String[] args) throws Exception {
        int mode = Integer.parseInt(args[0]);
        AllocationState.mode = mode;
        System.out.println(args.length > 1 && args[1].equals("innerPlain") ? AllocationLiveClient.innerPlain(mode)
            : args.length > 1 && args[1].equals("owned") ? AllocationLiveClient.owned(mode)
            : args.length > 1 && args[1].equals("loop") ? AllocationLiveClient.loop(mode)
            : args.length > 1 && args[1].equals("branch") ? AllocationLiveClient.branch(mode)
            : AllocationLiveClient.run(mode));
        System.out.println(AllocationState.trace);
    }
}
