class FinalMethodWriter {
    int value;
    FinalMethodWriter() { value = 3; }
    void reset() { value = 7; }
}
class FinalRepeated {
    int value;
    FinalRepeated() {
        value = 3;
        FinalContexts.note(value);
        value = 8;
        FinalContexts.note(value);
    }
}
class FinalLoop {
    static int value;
    static {
        for (int i = 0; i < 3; i++) {
            value = i;
            FinalContexts.note(value);
        }
    }
}
class FinalOnce {
    final int value;
    FinalOnce(int number) {
        while (true) {
            value = number;
            break;
        }
    }
}
class FinalBranch {
    final int value;
    FinalBranch(boolean choose) {
        if (choose) value = 3;
        else value = 5;
    }
}
public class FinalContexts {
    static String trace = "";
    static void note(int value) { trace += value + ":"; }
    public static void main(String[] args) {
        FinalMethodWriter writer = new FinalMethodWriter();
        writer.reset();
        System.out.println(writer.value);
        System.out.println(new FinalRepeated().value);
        System.out.println(FinalLoop.value);
        System.out.println(new FinalOnce(9).value);
        System.out.println(new FinalBranch(false).value);
        System.out.println(new FinalBranch(true).value);
        System.out.println(trace);
    }
}
