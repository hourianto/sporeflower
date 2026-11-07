public final class SharedExitDriver {
    public static void main(String[] args) {
        for (int mode = -1; mode <= 1; ++mode) {
            for (int mask = 0; mask < 32; ++mask) {
                EHSupport.trace = "";
                String outcome;
                try { outcome = "R" + SharedExceptionalExit.run(mode, mask); }
                catch (Throwable failure) { outcome = "E" + EHSupport.identity(failure) + ":" + failure.getClass().getName(); }
                System.out.println(mode + ":" + mask + ":" + outcome + ":" + EHSupport.trace);
            }
        }
    }
}
