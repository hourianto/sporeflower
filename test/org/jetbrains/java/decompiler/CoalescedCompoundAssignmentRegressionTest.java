package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;

public class CoalescedCompoundAssignmentRegressionTest extends DecompileRegressionTestBase {
  @Test
  void nestedCompoundAssignmentsKeepEvaluationOrderAndNarrowing() throws Exception {
    Path original = fixture.getTestDataDir().resolve("classes/java8");
    decompileClassFile(original.resolve("pkg/TestMixedCompoundAssignment.class"), "pkg/TestMixedCompoundAssignment.java");
    recompile();
    try (URLClassLoader before = loader(original);
         URLClassLoader after = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      Class<?> input = before.loadClass("pkg.TestMixedCompoundAssignment");
      Class<?> output = after.loadClass("pkg.TestMixedCompoundAssignment");
      Object inputObject = input.getConstructor().newInstance();
      Object outputObject = output.getConstructor().newInstance();
      Method inputMixed = input.getMethod("testNestedIntLongDouble", int.class, long.class, double.class);
      Method outputMixed = output.getMethod("testNestedIntLongDouble", int.class, long.class, double.class);
      Method inputIntegral = input.getMethod("testNestedLongIntLong", long.class, int.class, long.class);
      Method outputIntegral = output.getMethod("testNestedLongIntLong", long.class, int.class, long.class);
      int[] ints = {Integer.MIN_VALUE, -7, 0, 3, Integer.MAX_VALUE};
      long[] longs = {Long.MIN_VALUE, -13, 0, 5, Long.MAX_VALUE};
      double[] doubles = {-2.5, 0, 1.5, Double.NaN, Double.POSITIVE_INFINITY};
      for (int i : ints) {
        for (long j : longs) {
          for (double k : doubles) {
            assertEquals(invoke(inputMixed, inputObject, i, j, k), invoke(outputMixed, outputObject, i, j, k),
              () -> "mixed(" + i + ", " + j + ", " + k + ")");
          }
          for (long k : longs) {
            assertEquals(invoke(inputIntegral, inputObject, j, i, k), invoke(outputIntegral, outputObject, j, i, k),
              () -> "integral(" + j + ", " + i + ", " + k + ")");
          }
        }
      }
    }
  }

  private static URLClassLoader loader(Path classes) throws Exception {
    return new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader());
  }

  private static Object invoke(Method method, Object receiver, Object... arguments) throws Exception {
    try {
      return method.invoke(receiver, arguments);
    } catch (InvocationTargetException failure) {
      // Division by zero must occur on the same inputs after coalescing too.
      return failure.getCause().getClass();
    }
  }
}
