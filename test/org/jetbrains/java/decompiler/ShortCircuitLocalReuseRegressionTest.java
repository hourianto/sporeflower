package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.*;

public class ShortCircuitLocalReuseRegressionTest extends DecompileRegressionTestBase {
  @Test
  void reusesLocalsWithoutExpandingConditionsOrLeavingUnusedInitializers() throws Exception {
    Path input = writeSource("ShortCircuitReuse.java", """
      public class ShortCircuitReuse {
        public static int reads;

        private static String read(String[] tokens, int at) {
          reads++;
          return tokens[at];
        }

        public static int conditional(boolean stop, String[] tokens) {
          String token = read(tokens, 0);
          int total = token.length();
          if (stop) {
            total++;
          } else if (!stop && (token = read(tokens, 1)) != null) {
            total += token.length();
          }
          return total;
        }

        public static int loop(String[] tokens) {
          String token = null;
          int at = 0;
          int total = 0;
          while (at < tokens.length && (token = read(tokens, at++)) != null
              && !(token = token.substring(1)).isEmpty()) {
            total += token.length();
          }
          return total;
        }

        public static int nested(boolean first, int input) {
          int length = input;
          if ((first || (length = Math.abs(length)) > 0) && (length = Math.min(length, 12)) > 0) {
            return length;
          }
          return -1;
        }

        public static int bypass(boolean skip, int input) {
          int value = Math.abs(input);
          if (skip || (value = Math.min(value, 12)) == 0) {
            return value + 100;
          }
          return value;
        }
      }
      """);
    compileJava8NoDebug(input, outRoot());
    checkBehavior(outRoot());
    String source = decompileDirectory(outRoot(), "ShortCircuitReuse.java");
    recompile();
    checkBehavior(fixture.getTempDir().resolve("recompiled-out"));

    String conditional = method(source, "conditional");
    String loop = method(source, "loop");
    String nested = method(source, "nested");
    assertAll(source,
      () -> assertTrue(conditional.contains("else if ("), "Keep the else-if chain"),
      () -> assertReused(conditional, "String (var\\d+) = read\\(", "\\(%s = read\\("),
      () -> assertReused(loop, "String (var\\d+) = null;", "\\(%s = %s\\.substring\\(1\\)\\)"),
      () -> assertTrue(Pattern.compile("\\((var\\d+) = Math.abs\\(\\1\\)\\).*\\(\\1 = Math.min\\(\\1, 12\\)\\)")
        .matcher(nested).find(), "Keep one local through the nested condition"));
  }

  private static void assertReused(String method, String declaration, String use) {
    Matcher local = Pattern.compile(declaration).matcher(method);
    assertTrue(local.find(), method);
    String name = local.group(1);
    assertTrue(Pattern.compile(use.formatted(name, name)).matcher(method).find(), method);
  }

  private static String method(String source, String name) {
    int start = source.indexOf("public static int " + name + "(");
    assertTrue(start >= 0, source);
    int next = source.indexOf("public static", start + 1);
    return source.substring(start, next < 0 ? source.length() : next);
  }

  private static void checkBehavior(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("ShortCircuitReuse");
      var reads = type.getField("reads");
      var conditional = type.getMethod("conditional", boolean.class, String[].class);
      for (boolean stop : new boolean[]{false, true}) {
        for (String second : new String[]{null, "", "abcd"}) {
          reads.setInt(null, 0);
          assertEquals(3 + (stop ? 1 : second == null ? 0 : second.length()),
            conditional.invoke(null, stop, new String[]{"abc", second}));
          assertEquals(stop ? 1 : 2, reads.getInt(null), "short-circuit call count");
        }
      }

      var loop = type.getMethod("loop", String[].class);
      String[][] samples = {{}, {null}, {"a"}, {"abcd", "abc"}, {"abc", null, "ignored"}, {"abc", "a", "ignored"}};
      int[] totals = {0, 0, 0, 5, 2, 2};
      int[] counts = {0, 1, 1, 2, 2, 2};
      for (int i = 0; i < samples.length; i++) {
        reads.setInt(null, 0);
        assertEquals(totals[i], loop.invoke(null, (Object)samples[i]));
        assertEquals(counts[i], reads.getInt(null), "loop short-circuit call count");
      }

      var nested = type.getMethod("nested", boolean.class, int.class);
      var bypass = type.getMethod("bypass", boolean.class, int.class);
      for (boolean flag : new boolean[]{false, true}) {
        for (int value : new int[]{Integer.MIN_VALUE, -20, -1, 0, 1, 20, Integer.MAX_VALUE}) {
          int length = flag ? value : Math.abs(value);
          int expected = Math.min(length, 12) > 0 ? Math.min(length, 12) : -1;
          assertEquals(expected, nested.invoke(null, flag, value));
          int result = flag ? Math.abs(value) : Math.min(Math.abs(value), 12);
          assertEquals(result + (flag || result == 0 ? 100 : 0), bypass.invoke(null, flag, value));
        }
      }
    }
  }
}
