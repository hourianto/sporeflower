import pkg.*;

public class ConstructorLiveDriver {
    public static void main(String[] args) {
        for (int mode = 0; mode < 2; mode++) {
            for (int fail : new int[] { -1, 1, 3, 5 }) {
                TestConstructorPreludeEffects.events.clear();
                TestConstructorPreludeEffects.failingStep = fail;
                try {
                    TestConstructorUnliftablePrelude value = mode == 0
                        ? new TestConstructorUnliftablePrelude(true)
                        : new TestConstructorUnliftablePrelude("argument");
                    System.out.println(value.first + ":" + value.second + ":" + value.saved);
                } catch (RuntimeException failure) { System.out.println(failure.getMessage()); }
                System.out.println(TestConstructorPreludeEffects.events);
            }
        }
    }
}
