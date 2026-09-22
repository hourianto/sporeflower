package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.*;

public class LocalMergeScopeRegressionTest extends DecompileRegressionTestBase {
  @Test
  void findsEarlierCompatibleLocalsAndKeepsRequiredBindings() throws Exception {
    Path input = writeSource("LocalScopes.java", """
      import java.util.Arrays;

      public class LocalScopes {
        public static long mixed(int input) {
          long total = 0;
          { int value = input; total += value; }
          { byte value = (byte)input; total += value; }
          { int value = input + 1; total += value; }
          return total;
        }

        public static int caught(boolean fail) {
          int total = 0;
          { Exception error = new Exception("first"); total += error.getMessage().length(); }
          try {
            if (fail) throw new Exception("second");
          } catch (Exception error) {
            total += error.getMessage().length();
          }
          return total;
        }

        public static int each(String[] values) {
          int total = 0;
          {
            Object padding = values;
            String value = values[0];
            if (padding != null) total += value.length();
          }
          for (String value : Arrays.asList(values)) total += value.length();
          return total;
        }

        public static int caughtWithLocal(boolean fail) {
          int total = 0;
          try {
            Exception error = new Exception("body");
            total += error.getMessage().length();
            if (fail) throw error;
          } catch (Exception error) {
            total += error.getMessage().length();
          }
          return total;
        }
      }
      """);
    compileJava8NoDebug(input, outRoot());
    checkBehavior(outRoot());
    String source = decompileDirectory(outRoot(), "LocalScopes.java");
    recompile();
    checkBehavior(fixture.getTempDir().resolve("recompiled-out"));
    assertAll(source,
      () -> assertTrue(Pattern.compile("int (var\\d+) = var0;(?s:.*?)\\1 = var0 \\+ 1;").matcher(source).find(),
        "An intervening byte local must not hide the earlier int local"),
      () -> assertTrue(Pattern.compile("catch \\(Exception var\\d+\\)").matcher(source).find(), "Keep the catch binding"),
      () -> assertTrue(Pattern.compile("for \\(String var\\d+ : ").matcher(source).find(), "Keep the enhanced-for binding"));
  }

  private static void checkBehavior(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("LocalScopes");
      var mixed = type.getMethod("mixed", int.class);
      for (int value : new int[]{Integer.MIN_VALUE, -129, -1, 0, 1, 128, Integer.MAX_VALUE}) {
        assertEquals((long)value + (byte)value + (value + 1), mixed.invoke(null, value));
      }
      var caught = type.getMethod("caught", boolean.class);
      assertEquals(5, caught.invoke(null, false));
      assertEquals(11, caught.invoke(null, true));
      var caughtWithLocal = type.getMethod("caughtWithLocal", boolean.class);
      assertEquals(4, caughtWithLocal.invoke(null, false));
      assertEquals(8, caughtWithLocal.invoke(null, true));
      var each = type.getMethod("each", String[].class);
      assertEquals(7, each.invoke(null, (Object)new String[]{"abc", "d"}));
    }
  }
}
