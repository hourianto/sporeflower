/** Neutral oracle for initialization boundaries; no application or SDK code. */
public class AllocationGraphProbe {
    static boolean branchFlag;
    static boolean choiceFlag;
    static int limit;
    static int branchEffects;
    static int loopEffects;
    static int failedEffects;

    public static class Branch {
        public final int value;
        static { branchFlag = true; }
        public Branch(int value) { this.value = value; }
    }

    public static class Loop {
        public final int value;
        static { limit = 3; }
        public Loop(int value) { this.value = value; }
    }

    public static class Choice {
        public final int value;
        static { choiceFlag = true; }
        public Choice(int value) { this.value = 100 + value; }
        public Choice(String value) { this.value = 200 + value.length(); }
    }

    public static class Failed {
        static { if (failInitialization()) throw new RuntimeException("init"); }
        public Failed(int ignored) { }
    }

    static boolean failInitialization() { return true; }
    static int effect(int value) { branchEffects += value; return value; }
    static void effectLoop() { loopEffects++; }
    static int effectFailed() { failedEffects++; return 7; }

    public static Branch branch() {
        return new Branch(branchFlag ? effect(10) : effect(20));
    }

    // generate.py moves this NEW ahead of the loop, retaining one allocation
    // token across its backedge. Java has no statement syntax for that layout.
    public static Loop loop() {
        int value = 0;
        while (value < limit) { effectLoop(); value++; }
        return new Loop(value);
    }

    // generate.py gives both constructor calls the same preceding NEW token.
    public static Choice alternatives() {
        if (choiceFlag) return new Choice(1);
        return new Choice("two");
    }

    public static int failed() {
        try {
            new Failed(branchFlag ? effectFailed() : 9);
            return -1;
        } catch (ExceptionInInitializerError expected) {
            return failedEffects;
        }
    }

    public static void main(String[] args) {
        System.out.println(branch().value);
        System.out.println(branchEffects);
        System.out.println(loop().value);
        System.out.println(loopEffects);
        System.out.println(alternatives().value);
        System.out.println(failed());
    }
}
