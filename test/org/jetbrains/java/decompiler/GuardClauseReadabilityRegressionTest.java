package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.nio.file.Path;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.*;

public class GuardClauseReadabilityRegressionTest extends DecompileRegressionTestBase {
  @Test
  void staticInitializersKeepConditionalBodiesInsteadOfReturning() throws Exception {
    Path input = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestStaticInitializerGuard.class");
    decompileClassFile(input, "pkg/TestStaticInitializerGuard.java");
    recompile();
    String previous = System.getProperty("guard.clause.fixture");
    try {
      for (boolean enabled : new boolean[]{false, true}) {
        System.setProperty("guard.clause.fixture", Boolean.toString(enabled));
        // Each setting must initialize a fresh pair of classes.
        try (BehaviorComparison behavior = compareBehavior(input.getParent().getParent())) {
          behavior.assertSame("static initializer enabled=" + enabled, enabled ? 7 : 130, loader ->
            loader.loadClass("pkg.TestStaticInitializerGuard").getField("value").getInt(null));
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
    try (BehaviorComparison behavior = compareBehavior()) {
      for (int flags = 0; flags < 8; flags++) {
        boolean enabled = (flags & 1) != 0, guard = (flags & 2) != 0, touch = (flags & 4) != 0;
        for (int seed : new int[]{Integer.MIN_VALUE, -1, 0, 1, Integer.MAX_VALUE}) {
          behavior.assertSame("flags=" + flags + ", seed=" + seed, loader -> {
            int[] values = {seed, seed + 1, -seed};
            loader.loadClass("GuardClause").getMethod("update", boolean.class, boolean.class, boolean.class, int[].class)
              .invoke(null, enabled, guard, touch, values);
            return values;
          });
        }
      }
    }
    assertFalse(source.contains("= null"), source);
    assertTrue(source.contains("return;"), source);
    assertTrue(Pattern.compile("(?m)^      for ").matcher(source).find(), source);
  }

}
