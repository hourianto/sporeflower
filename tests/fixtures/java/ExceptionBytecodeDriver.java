import pkg.*;
public class ExceptionBytecodeDriver {
    public static void main(String[] args) throws Exception {
        for (int method = 0; method < 3; method++) {
            for (int flags = 0; flags < 16; flags++) {
                for (int failure = 0; failure <= 8; failure++) {
                    for (int cleanupFailure = 0; cleanupFailure <= 6; cleanupFailure += 3) {
                        FinallyReturnState.stream = (flags & 1) != 0 ? new Object() : null;
                        FinallyReturnState.connection = (flags & 2) != 0;
                        FinallyReturnState.success = (flags & 4) != 0;
                        FinallyReturnState.error = (flags & 8) != 0;
                        FinallyReturnState.failAt = failure;
                        FinallyReturnState.cleanupFailAt = cleanupFailure;
                        FinallyReturnState.trace = "";
                        String result = "void";
                        try {
                            if (method == 0) TestFinallyReturnRange.run();
                            else if (method == 1) TestFinallyDistinctContinuation.run();
                            else result = String.valueOf(TestFinallyCleanupCatch.run());
                        } catch (Throwable exception) { result = exception.getClass().getName() + ":" + exception.getMessage(); }
                        System.out.println(result + ":" + FinallyReturnState.trace);
                    }
                }
            }
        }
        for (int method = 0; method < 4; method++) {
            for (int flags = 0; flags < 4; flags++) {
                for (int mode = 0; mode < 5; mode++) {
                    ExceptionDispatchState.cleanups = ExceptionDispatchState.catches = ExceptionDispatchState.tails = 0;
                    ExceptionDispatchState.failCleanup = (flags & 1) != 0;
                    ExceptionDispatchState.escaped = null;
                    String result;
                    try {
                        int value;
                        if (method == 0) value = TestShadowedCatchAll.run(mode);
                        else if (method == 1) value = TestShadowedCatchAll.handled(mode);
                        else if (method == 2) value = TestShadowedCatchAll.separated(mode);
                        else value = TestSharedHandlerRethrow.run(mode, (flags & 2) != 0);
                        result = String.valueOf(value);
                    } catch (Throwable exception) {
                        result = exception.getClass().getName() + ":" + exception.getMessage() + ":identity=" + (exception == ExceptionDispatchState.escaped);
                    }
                    System.out.println(result + ":" + ExceptionDispatchState.cleanups + ":" + ExceptionDispatchState.catches + ":" + ExceptionDispatchState.tails);
                }
            }
        }
    }
}
