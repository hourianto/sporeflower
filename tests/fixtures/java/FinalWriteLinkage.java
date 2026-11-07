class FinalWriteState {
    static String trace = "";
    static int mode;
}
class FinalWriteOwner {
    public int value;
    public static int shared;
    static {
        FinalWriteState.trace += "init;";
        if (FinalWriteState.mode == 2) throw new RuntimeException("initialization");
        shared = 3;
    }
    FinalWriteOwner() { value = 5; }
}
class ConstantFinalOwner {
    public static int value;
    public static final int seed = 4;
    static {
        FinalWriteState.trace += "constant-init;";
        if (FinalWriteState.mode == 2) throw new RuntimeException("initialization");
    }
}
interface ConstantFinalInterface {
    int value = 4;
    int ready = ConstantFinalEffects.initialize();
}
class ConstantFinalEffects {
    static int initialize() {
        FinalWriteState.trace += "interface-init;";
        if (FinalWriteState.mode == 2) throw new RuntimeException("initialization");
        return 8;
    }
}
class LegacyFinalWriter {
    public int value;
    public static int shared;
    LegacyFinalWriter() { value = 5; }
    static { shared = 3; }
    int write() { return value = 7; }
    static int writeStatic() { return shared = 9; }
}
class ModernFinalWriter {
    public int value;
    public static int shared;
    ModernFinalWriter() { value = 5; }
    static { shared = 3; }
    int write() { return value = 7; }
    static int writeStatic() { return shared = 9; }
}
class FinalWriteCaller implements ConstantFinalInterface {
    static FinalWriteOwner receiver() {
        FinalWriteState.trace += "receiver;";
        return FinalWriteState.mode == 0 ? null : new FinalWriteOwner();
    }
    static int argument() {
        FinalWriteState.trace += "argument;";
        if (FinalWriteState.mode == 3) throw new IllegalArgumentException("argument");
        return FinalWriteState.mode == 4 ? 0 : 2;
    }
    static int direct() { return receiver().value = argument(); }
    static int compound() { return receiver().value += argument(); }
    static int divide() { return receiver().value /= argument(); }
    static int increment() { return receiver().value++; }
    static int decrement() { return --receiver().value; }
    static int directStatic() { return FinalWriteOwner.shared = argument(); }
    static int compoundStatic() { return FinalWriteOwner.shared += argument(); }
    static int divideStatic() { return FinalWriteOwner.shared /= argument(); }
    static int incrementStatic() { return FinalWriteOwner.shared++; }
    static int readConstant() { return ConstantFinalOwner.value; }
    static int guardedConstant() { return FinalWriteState.mode == 0 ? 9 : ConstantFinalOwner.value; }
    static int ignoredConstant() { int ignored = ConstantFinalOwner.value; return 7; }
    static int compoundConstant() { return ConstantFinalOwner.value += argument(); }
    static int divideConstant() { return ConstantFinalOwner.value /= argument(); }
    static int incrementConstant() { return ConstantFinalOwner.value++; }
    static int readInterface() { return ConstantFinalInterface.ready; }
}
public class FinalWriteLinkage {
    public static void main(String[] args) {
        FinalWriteState.mode = Integer.parseInt(args[1]);
        try {
            int value;
            if (args[0].equals("direct")) value = FinalWriteCaller.direct();
            else if (args[0].equals("compound")) value = FinalWriteCaller.compound();
            else if (args[0].equals("divide")) value = FinalWriteCaller.divide();
            else if (args[0].equals("increment")) value = FinalWriteCaller.increment();
            else if (args[0].equals("decrement")) value = FinalWriteCaller.decrement();
            else if (args[0].equals("directStatic")) value = FinalWriteCaller.directStatic();
            else if (args[0].equals("compoundStatic")) value = FinalWriteCaller.compoundStatic();
            else if (args[0].equals("divideStatic")) value = FinalWriteCaller.divideStatic();
            else if (args[0].equals("incrementStatic")) value = FinalWriteCaller.incrementStatic();
            else if (args[0].equals("readConstant")) value = FinalWriteCaller.readConstant();
            else if (args[0].equals("guardedConstant")) value = FinalWriteCaller.guardedConstant();
            else if (args[0].equals("ignoredConstant")) value = FinalWriteCaller.ignoredConstant();
            else if (args[0].equals("compoundConstant")) value = FinalWriteCaller.compoundConstant();
            else if (args[0].equals("divideConstant")) value = FinalWriteCaller.divideConstant();
            else if (args[0].equals("incrementConstant")) value = FinalWriteCaller.incrementConstant();
            else if (args[0].equals("readInterface")) value = FinalWriteCaller.readInterface();
            else if (args[0].equals("legacy")) value = new LegacyFinalWriter().write();
            else if (args[0].equals("legacyStatic")) value = LegacyFinalWriter.writeStatic();
            else if (args[0].equals("modern")) value = new ModernFinalWriter().write();
            else value = ModernFinalWriter.writeStatic();
            System.out.println(value);
        } catch (Throwable failure) { System.out.println(failure.getClass().getName()); }
        System.out.println(FinalWriteState.trace);
    }
}
