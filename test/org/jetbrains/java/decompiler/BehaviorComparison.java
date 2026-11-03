package org.jetbrains.java.decompiler;

import java.io.IOException;
import java.lang.reflect.InvocationTargetException;
import java.net.URLClassLoader;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.Objects;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

/** Two isolated programs, kept alive together so observations can include state across calls. */
final class BehaviorComparison implements AutoCloseable {
  private final URLClassLoader original;
  private final URLClassLoader rebuilt;

  BehaviorComparison(Path originalClasses, Path rebuiltClasses) throws IOException {
    original = DecompileRegressionTestBase.loader(originalClasses);
    try {
      rebuilt = DecompileRegressionTestBase.loader(rebuiltClasses);
    } catch (IOException failure) {
      original.close();
      throw failure;
    }
  }

  @FunctionalInterface
  interface Observation {
    Object observe(ClassLoader loader) throws Exception;
  }

  /**
   * Run the same observation in each program. Create mutable inputs inside the
   * callback and return values, arrays or field snapshots with loader-independent
   * equality. Assertions in the callback can also establish the fixture's oracle.
   */
  void assertSame(String context, Observation observation) throws Exception {
    Outcome expected = observe(original, observation);
    Outcome actual = observe(rebuilt, observation);
    assertSame(context, expected, actual);
  }

  /** Exception diagnostics can contain loader identities or rewritten expressions. */
  void assertSameIncludingExceptionMessage(String context, Observation observation) throws Exception {
    Outcome expected = observe(original, observation);
    Outcome actual = observe(rebuilt, observation);
    assertSame(context, expected, actual);
    if (expected.failure != null) {
      assertEquals(expected.failure.message, actual.failure.message, context + ": exception message");
    }
  }

  /** Mutable inputs must instead be created separately inside an observation. */
  void assertSameStaticCall(String className, String methodName, Class<?>[] parameterTypes, Object... arguments) throws Exception {
    assertTrue(Arrays.stream(arguments).allMatch(value -> value == null || value instanceof String
      || value instanceof Boolean || value instanceof Character || value instanceof Byte || value instanceof Short
      || value instanceof Integer || value instanceof Long || value instanceof Float || value instanceof Double),
      "Mutable inputs require an observation callback");
    assertSame(className + "." + methodName + Arrays.deepToString(arguments), loader ->
      loader.loadClass(className).getMethod(methodName, parameterTypes).invoke(null, arguments));
  }

  /** Also verify the original fixture against an independent expected value. */
  void assertSame(String context, Object expectedValue, Observation observation) throws Exception {
    Outcome expected = observe(original, observation);
    assertSame(context + ": original fixture", new Outcome(expectedValue, null), expected);
    assertSame(context, expected, observe(rebuilt, observation));
  }

  private static void assertSame(String context, Outcome expected, Outcome actual) {
    assertEquals(expected.failure == null ? null : expected.failure.type,
      actual.failure == null ? null : actual.failure.type, context + ": thrown exception");
    assertTrue(Objects.deepEquals(expected.value, actual.value),
      () -> context + ": expected " + Arrays.deepToString(new Object[]{expected.value})
        + ", got " + Arrays.deepToString(new Object[]{actual.value}));
  }

  private static Outcome observe(ClassLoader loader, Observation observation) throws Exception {
    try {
      return new Outcome(observation.observe(loader), null);
    } catch (InvocationTargetException failure) {
      // Compare application failures across loaders, but let reflection setup
      // errors and assertion failures fail the test instead of matching them.
      Throwable cause = failure.getCause();
      return new Outcome(null, new Failure(cause.getClass().getName(), cause.getMessage()));
    }
  }

  private record Outcome(Object value, Failure failure) {}
  private record Failure(String type, String message) {}

  @Override
  public void close() throws IOException {
    try (original; rebuilt) {
      // Both loaders close even if the first close fails.
    }
  }
}
