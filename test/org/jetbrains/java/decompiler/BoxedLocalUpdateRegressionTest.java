package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;

public class BoxedLocalUpdateRegressionTest extends DecompileRegressionTestBase {
  @Test
  void elidedBoxingKeepsAssignmentPrecedenceAndEvaluationOrder() throws Exception {
    Path input = writeSource("BoxedUpdates.java", """
      public class BoxedUpdates {
        public static int add(int start) {
          Integer value = start;
          int result = ++value + ++value;
          return result * 100 + value;
        }
        public static int triple(int start) {
          Integer value = start;
          int result = ++value + ++value + ++value;
          return result * 100 + value;
        }
        public static int multiply(int start) {
          Integer value = start;
          int result = ++value * --value;
          return result * 100 + value;
        }
        public static int cast(int start) {
          Integer value = start;
          int result = ((Integer)(Object)(value = value + 1)).intValue() + ++value;
          return result * 100 + value;
        }
      }
      """);
    compileJava8NoDebug(input, outRoot());
    checkBehavior(outRoot());
    decompileDirectory(outRoot(), "BoxedUpdates.java");
    recompile();
    checkBehavior(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void checkBehavior(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("BoxedUpdates");
      for (int start : new int[]{Integer.MIN_VALUE, -5, 0, 3, 127, Integer.MAX_VALUE}) {
        int two = ((start + 1) + (start + 2)) * 100 + (start + 2);
        int three = ((start + 1) + (start + 2) + (start + 3)) * 100 + (start + 3);
        int product = ((start + 1) * start) * 100 + start;
        assertEquals(two, type.getMethod("add", int.class).invoke(null, start));
        assertEquals(three, type.getMethod("triple", int.class).invoke(null, start));
        assertEquals(product, type.getMethod("multiply", int.class).invoke(null, start));
        assertEquals(two, type.getMethod("cast", int.class).invoke(null, start));
      }
    }
  }
}
