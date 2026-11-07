package pkg;

import java.util.ArrayList;
import java.util.List;

public class TestConstructorPreludeEffects {
    public static final List<Integer> events = new ArrayList<Integer>();
    public static int failingStep = -1;
    public final int first;
    public final int second;
    public TestConstructorPreludeEffects(int first, int second) {
        record(5);
        this.first = first;
        this.second = second;
    }
    public static void record(int step) {
        events.add(Integer.valueOf(step));
        if (step == failingStep) throw new IllegalStateException("step " + step);
    }
    public static int value(int step) { record(step); return events.size(); }
}
