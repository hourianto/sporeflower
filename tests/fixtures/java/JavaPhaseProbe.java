public class JavaPhaseProbe {
    public static void main(String[] args) {
        try {
            if (args[0].equals("interface")) {
                System.out.println(PhaseInterface.VALUE);
            } else {
                System.out.println(PhaseCaller.run());
            }
        } catch (Throwable failure) {
            System.out.println(failure.getClass().getName());
        }
        System.out.println(PhaseState.trace);
    }
}
interface PhaseInterface {
    int FIRST = PhaseState.step(1);
    int VALUE = PhasePlainTarget.present(PhaseState.step(2));
    int LAST = PhaseState.step(3);
}
class PhaseState {
    static int trace;
    static int step(int value) { trace = trace * 10 + value; return value; }
}
class PhasePlainTarget {
    static int present(int value) { return value; }
}
class PhaseCheckedTarget {
    static int present(int value) throws java.io.IOException { return value; }
}
class PhaseCaller {
    static int run() {
        try { return PhaseCheckedTarget.present(PhaseState.step(4)); }
        catch (java.io.IOException exception) { return PhaseState.step(5); }
    }
}
