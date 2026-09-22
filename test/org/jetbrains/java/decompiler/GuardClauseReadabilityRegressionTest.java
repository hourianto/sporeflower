package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.*;

public class GuardClauseReadabilityRegressionTest extends DecompileRegressionTestBase {
  @Test
  void staticInitializersKeepConditionalBodiesInsteadOfReturning() throws Exception {
    Path input = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestStaticInitializerGuard.class");
    String source = decompileClassFile(input, "pkg/TestStaticInitializerGuard.java");
    recompile();
    String previous = System.getProperty("guard.clause.fixture");
    try {
      for (boolean enabled : new boolean[]{false, true}) {
        System.setProperty("guard.clause.fixture", Boolean.toString(enabled));
        for (Path classes : java.util.List.of(input.getParent().getParent(), fixture.getTempDir().resolve("recompiled-out"))) {
          try (URLClassLoader loader = loader(classes)) {
            assertEquals(enabled ? 7 : 130, loader.loadClass("pkg.TestStaticInitializerGuard").getField("value").getInt(null), source);
          }
        }
      }
    } finally {
      if (previous == null) System.clearProperty("guard.clause.fixture");
      else System.setProperty("guard.clause.fixture", previous);
    }
  }

  @Test
  void removingDeadStoresKeepsTheMethodTailOutsideTheGuard() throws Exception {
    Path input = writeSource("GuardClause.java", """
      public class GuardClause {
        public static void update(boolean enabled, boolean guard, boolean touch, int[] values) {
          if (enabled) {
            Object unused = null;
            if (guard) {
              if (touch) values[0]++;
              return;
            }
          }
          for (int i = 0; i < values.length; i++) values[i] = values[i] * 3 + 1;
          values[0]++;
          for (int i = values.length - 1; i >= 0; i--) values[i] ^= values[0];
        }
      }
      """);
    compileJava8NoDebug(input, outRoot());
    String source = decompileDirectory(outRoot(), "GuardClause.java");
    recompile();
    try (URLClassLoader original = loader(outRoot());
         URLClassLoader rebuilt = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      var before = original.loadClass("GuardClause").getMethod("update", boolean.class, boolean.class, boolean.class, int[].class);
      var after = rebuilt.loadClass("GuardClause").getMethod("update", boolean.class, boolean.class, boolean.class, int[].class);
      for (int flags = 0; flags < 8; flags++) {
        for (int seed : new int[]{Integer.MIN_VALUE, -1, 0, 1, Integer.MAX_VALUE}) {
          int[] expected = {seed, seed + 1, -seed}, actual = expected.clone();
          before.invoke(null, (flags & 1) != 0, (flags & 2) != 0, (flags & 4) != 0, expected);
          after.invoke(null, (flags & 1) != 0, (flags & 2) != 0, (flags & 4) != 0, actual);
          assertArrayEquals(expected, actual);
        }
      }
    }
    assertFalse(source.contains("= null"), source);
    assertTrue(source.contains("return;"), source);
    assertTrue(Pattern.compile("(?m)^      for ").matcher(source).find(), source);
  }

  private static URLClassLoader loader(Path classes) throws Exception {
    return new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader());
  }
}
