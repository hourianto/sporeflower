package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;
import java.net.URL;
import java.net.URLClassLoader;
import java.lang.reflect.InvocationTargetException;
import java.nio.file.Path;
import static org.junit.jupiter.api.Assertions.assertEquals;

public class BooleanComparisonSemanticsRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testComparisonRemovalPreservesEvaluationAndNumericEquality() throws Exception {
    Path source = writeSource("pkg/Comparisons.java", """
      package pkg;
      public class Comparisons {
        public static int calls;
        static boolean tick(boolean fail) { calls++; if (fail) throw new IllegalStateException(); return true; }
        public static boolean and(int x, boolean fail) { return x != 1 && tick(fail) && x == 2; }
        public static boolean or(int x, boolean fail) { return x == 1 || tick(fail) || x != 2; }
        public static boolean division(int x, boolean fail) { return x != 1 && 1 / (x - 1) == 0 && x == 2; }
        public static boolean reassignment(int x, boolean fail) { return x == 1 && (x = 2) > 0 && x != 2; }
        public static boolean zeroFloat(float x) { return x == 0.0f && x != -0.0f; }
        public static boolean zeroDouble(double x) { return x == 0.0 && x != -0.0; }
        public static boolean mixed(long x) { return x == 9007199254740993L && x != 9007199254740992.0; }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    decompileDirectory(outRoot(), "pkg/Comparisons.java");
    recompile();
    try (URLClassLoader original = loader(outRoot());
         URLClassLoader rebuilt = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      Class<?> first = original.loadClass("pkg.Comparisons"), second = rebuilt.loadClass("pkg.Comparisons");
      for (String name : new String[]{"and", "or", "division", "reassignment"}) {
        for (int x = -1; x <= 3; x++) for (boolean fail : new boolean[]{false, true}) {
          compare(first, second, name, new Class<?>[]{int.class, boolean.class}, x, fail);
        }
      }
      for (float x : new float[]{0.0f, -0.0f, 1, -1, Float.NaN, Float.POSITIVE_INFINITY, Float.NEGATIVE_INFINITY}) {
        compare(first, second, "zeroFloat", new Class<?>[]{float.class}, x);
        compare(first, second, "zeroDouble", new Class<?>[]{double.class}, (double)x);
      }
      for (long x : new long[]{0, 9007199254740992L, 9007199254740993L, 9007199254740994L}) {
        compare(first, second, "mixed", new Class<?>[]{long.class}, x);
      }
    }
  }

  private static URLClassLoader loader(Path classes) throws Exception {
    return new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader());
  }

  private static void compare(Class<?> original, Class<?> rebuilt, String name, Class<?>[] types, Object... args) throws Exception {
    assertEquals(outcome(original, name, types, args), outcome(rebuilt, name, types, args), name + java.util.Arrays.toString(args));
  }

  private static String outcome(Class<?> type, String name, Class<?>[] types, Object[] args) throws Exception {
    type.getField("calls").set(null, 0);
    String result;
    try {
      result = "result:" + type.getMethod(name, types).invoke(null, args);
    } catch (InvocationTargetException exception) {
      result = "throw:" + exception.getCause().getClass().getName();
    }
    return result + ", calls:" + type.getField("calls").get(null);
  }
}
