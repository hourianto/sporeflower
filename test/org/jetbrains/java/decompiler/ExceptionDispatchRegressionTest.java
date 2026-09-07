package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.lang.reflect.InvocationTargetException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;

public class ExceptionDispatchRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void shadowedCatchAllDoesNotCatchCleanupFailures() throws Exception {
    compileJava8NoDebug(writeSource("pkg/ExceptionDispatchState.java", """
      package pkg;
      public class ExceptionDispatchState {
        public static int cleanups;
        public static int catches;
        public static int tails;
        public static boolean failCleanup;
        public static int body(int mode) {
          if (mode == 1) throw new IllegalArgumentException("body");
          if (mode == 2) throw new AssertionError("body");
          return 9;
        }
        public static void cleanup() {
          cleanups++;
          if (failCleanup) throw new IllegalStateException("cleanup");
        }
        public static void caught(int mode) {
          catches++;
          if (mode == 3) throw new UnsupportedOperationException("catch");
        }
        public static int finish(int mode, int value) {
          if (mode == 3) throw new UnsupportedOperationException("second body");
          return value + 7;
        }
        public static void tail() { tails++; }
        public static void unreachable() { throw new AssertionError("shadowed handler"); }
      }
      """), outRoot());
    for (String name : new String[]{"TestShadowedCatchAll", "TestSharedHandlerRethrow"}) {
      Files.copy(fixture.getTestDataDir().resolve("classes/jasm/pkg/" + name + ".class"),
        outRoot().resolve("pkg/" + name + ".class"));
    }
    checkBehavior(outRoot());
    String source = decompileDirectory(outRoot(), "pkg/TestShadowedCatchAll.java");
    assertFalse(source.contains("Couldn't be decompiled"), source);
    recompile();
    checkBehavior(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void checkBehavior(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> state = loader.loadClass("pkg.ExceptionDispatchState");
      for (String name : new String[]{"run", "handled", "separated"}) {
        boolean handled = name.equals("handled");
        var run = loader.loadClass("pkg.TestShadowedCatchAll").getMethod(name, int.class);
        Object[] normalResults = switch (name) {
          case "run" -> new Object[]{9, 0, 0};
          case "handled" -> new Object[]{9, 0, 0, UnsupportedOperationException.class};
          default -> new Object[]{9, -1, AssertionError.class, -1, 16};
        };
        for (boolean failCleanup : new boolean[]{false, true}) {
          for (int mode = 0; mode < normalResults.length; mode++) {
            state.getField("cleanups").setInt(null, 0);
            state.getField("catches").setInt(null, 0);
            state.getField("failCleanup").setBoolean(null, failCleanup);
            Object result;
            try {
              result = run.invoke(null, mode);
            } catch (InvocationTargetException exception) {
              result = exception.getCause().getClass();
            }
            Object expected = failCleanup ? IllegalStateException.class : normalResults[mode];
            assertEquals(expected, result, name + ": mode=" + mode + ", failCleanup=" + failCleanup);
            assertEquals(1, state.getField("cleanups").getInt(null));
            assertEquals(handled && mode != 0 ? 1 : 0, state.getField("catches").getInt(null));
          }
        }
      }
      var shared = loader.loadClass("pkg.TestSharedHandlerRethrow").getMethod("run", int.class, boolean.class);
      for (boolean failCleanup : new boolean[]{false, true}) {
        for (boolean tail : new boolean[]{false, true}) {
          for (int mode = 0; mode < 4; mode++) {
            state.getField("cleanups").setInt(null, 0);
            state.getField("tails").setInt(null, 0);
            state.getField("failCleanup").setBoolean(null, failCleanup);
            Object result;
            try {
              result = shared.invoke(null, mode, tail);
            } catch (InvocationTargetException exception) {
              result = exception.getCause().getClass();
            }
            boolean failed = mode == 1 || mode == 2;
            Object expected = !failed ? 9 : failCleanup ? IllegalStateException.class
              : mode == 1 ? IllegalArgumentException.class : AssertionError.class;
            assertEquals(expected, result);
            assertEquals(failed ? 1 : 0, state.getField("cleanups").getInt(null));
            assertEquals(failed && tail && !failCleanup ? 1 : 0, state.getField("tails").getInt(null));
          }
        }
      }
    }
  }
}
