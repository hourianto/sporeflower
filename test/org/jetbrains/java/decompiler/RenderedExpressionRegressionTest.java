package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.*;

class RenderedExpressionRegressionTest extends DecompileRegressionTestBase {
  @Test
  void emittedBoxingAndCastsComposeWithOperatorsQualifiersAndOverloads() throws Exception {
    Path input = writeSource("ExpressionForms.java", """
      public class ExpressionForms {
        static int choose(int value) { return 1; }
        static int choose(Integer value) { return 2; }
        public static int overloaded(int input) {
          Integer value = input;
          return choose(Integer.valueOf(value = value + 1)) * 100 + choose(value.intValue());
        }
        public static int qualifier(int input) {
          int value = input;
          return Integer.valueOf(value = value + 1).toString().length() * 100 + value;
        }
        public static int narrowing(int input) {
          Integer value = input;
          int result = (byte)Integer.valueOf(value = value + 256).intValue() + ++value;
          return result * 100 + value;
        }
        public static boolean condition(int input) {
          Boolean value = input > 0;
          return !(value = !value) && (value = !value);
        }
        public static int array(int input) {
          Integer value = input;
          return new Integer[]{++value}[0] * 100 + ++value;
        }
        public static long unaryCast(int input) {
          return (long)-input - input;
        }
        public static long postfixCast(int input) {
          long value = input;
          int result = (int)value++;
          return result * 100L + value;
        }
      }
      """);
    compileJava8NoDebug(input, outRoot());
    String source = decompileDirectory(outRoot(), "ExpressionForms.java");
    recompile();
    try (URLClassLoader original = loader(outRoot());
         URLClassLoader rebuilt = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      Class<?> before = original.loadClass("ExpressionForms"), after = rebuilt.loadClass("ExpressionForms");
      for (String name : new String[]{"overloaded", "qualifier", "narrowing", "condition", "array", "unaryCast", "postfixCast"}) {
        for (int value : new int[]{Integer.MIN_VALUE, -129, -1, 0, 127, Integer.MAX_VALUE}) {
          assertEquals(before.getMethod(name, int.class).invoke(null, value), after.getMethod(name, int.class).invoke(null, value),
            name + "(" + value + ")\n" + source);
        }
      }
    }
    String qualifier = source.substring(source.indexOf("public static int qualifier("), source.indexOf("public static int narrowing("));
    assertFalse(qualifier.contains("(Integer.valueOf("), "An explicit boxing call is already a valid qualifier:\n" + qualifier);
  }

  private static URLClassLoader loader(Path path) throws Exception {
    return new URLClassLoader(new URL[]{path.toUri().toURL()}, ClassLoader.getPlatformClassLoader());
  }
}
