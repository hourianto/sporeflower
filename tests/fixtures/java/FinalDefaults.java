public class FinalDefaults {
    final int number;
    final Object label;

    public FinalDefaults() { number = 0; label = null; }
    public FinalDefaults(int number) { this.number = number; label = "written"; }
    public FinalDefaults(boolean flag) { this(flag ? 11 : 22); }

    public static void main(String[] args) {
        FinalDefaults[] values = {new FinalDefaults(), new FinalDefaults(7), new FinalDefaults(true)};
        for (int i = 0; i < values.length; i++) {
            System.out.println(values[i].number + ":" + values[i].label);
        }
    }
}
