package pkg;

public class TestConstructorPreludeBase {
  public static int calls;
  public static RuntimeException failure;

  public TestConstructorPreludeBase() {
    calls++;
    if (failure != null) {
      throw failure;
    }
  }
}
