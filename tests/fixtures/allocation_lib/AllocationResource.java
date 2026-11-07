public class AllocationResource {
    public final int id;
    public AllocationResource(int id) {
        AllocationState.trace += "resource;";
        this.id = id;
    }
}
