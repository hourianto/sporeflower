public class MonitorCopiesDriver {
    public static void main(String[] arguments) {
            for (int mask = 0; mask < 8; mask++) {
                EHSupport.trace = "";
                String outcome = "return";
                try {
                    MonitorCopies.run(EHSupport.monitor, mask);
                } catch (Throwable failure) {
                    outcome = "throw:" + EHSupport.identity(failure) + ":" + failure.getClass().getName();
                }
                System.out.println(mask + ":" + outcome + ":" + EHSupport.trace + ":" + Thread.holdsLock(EHSupport.monitor));
            }
    }
}
