package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

import java.lang.reflect.InvocationTargetException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class FinallyValueFlowTest extends DecompileRegressionTestBase {
  @ParameterizedTest
  @ValueSource(booleans = {true, false})
  public void writesAndEvaluationOrderSurviveFinallyContinuations(boolean deinline) throws Exception {
    DecompilerContext.setProperty(IFernflowerPreferences.FINALLY_DEINLINE, deinline ? "1" : "0");
    String source = compileDecompileAndRead("pkg/FinallyValues.java", """
      package pkg;
      public class FinallyValues {
        public static int observed;
        static class Resource {
          final int value;
          Resource(int value) { this.value = value; }
          void close() {
            observed++;
            if (value == 1) throw new IllegalStateException("close");
          }
        }
        public static int clearResource(int x) {
          Resource resource = null;
          try {
            resource = new Resource(x);
            if (x < 0) return x;
            return x + 1;
          } finally {
            if (resource != null) resource.close();
            resource = null;
          }
        }
        public static int overwrite(int x) {
          int value = x;
          exit: {
            try {
              if (x < 0) break exit;
              value = 10;
            } finally {
              value = x + 20;
            }
          }
          return value;
        }
        public static int conditional(int x) {
          int value = x;
          exit: {
            try {
              if (x < 0) break exit;
              value = 10;
            } finally {
              if (x != 0) value = 20;
              observed = value;
            }
          }
          return value;
        }
        public static int conditionalDefinition(int x) {
          int value;
          exit: {
            try {
              value = 10;
              if (x < 0) break exit;
              value = 20;
            } finally {
              if (x == 0) value = 30;
            }
          }
          return value;
        }
        public static int returnOutsideCatch(int x) {
          exit: {
            try {
              if (x <= 0) break exit;
              observed = 10;
            } catch (RuntimeException failure) {
              observed = 99;
              return -1;
            }
          }
          return 100 / x;
        }
        public static int throwOutsideCatch(int x) {
          RuntimeException pending = new IllegalStateException("pending");
          exit: {
            try {
              if (x <= 0) break exit;
              observed = 10;
            } catch (RuntimeException failure) {
              observed = 99;
              return -1;
            }
          }
          throw pending;
        }
        public static int nested(int x) {
          int value = x;
          exit: {
            try {
              try {
                if (x < 0) break exit;
                value = 10;
              } finally {
                if (x != 0) value += 20;
              }
            } finally {
              observed = value;
              value += 30;
            }
          }
          return value;
        }
        public static int loop(int x) {
          int value = x;
          for (int i = 0; i < 3; i++) {
            try {
              if (x == i) break;
              value += i;
            } finally {
              if (i != 1) value += 10;
              observed += value;
            }
          }
          return value;
        }
        public static int returnBeforeCleanup(int x) {
          int value = x;
          try {
            if (x < 0) return value;
            value += 10;
          } finally {
            value += 20;
            observed = value;
          }
          return value;
        }
        public static int returnAfterCleanup(int x) {
          exit: {
            try {
              if (x < 0) break exit;
              observed = 10;
            } finally {
              observed += 20;
            }
          }
          return 100 / observed;
        }
        public static int throwAfterCleanup(int x) {
          RuntimeException failure = new RuntimeException("pending");
          try {
            observed = x;
            if (x == 0) throw new IllegalStateException("body");
          } catch (Throwable caught) {
            observed = 99;
            throw new IllegalArgumentException("catch");
          } finally {
            throw failure;
          }
        }
        public static int unboxAfterCleanup(int x) {
          Integer value = x == 0 ? null : Integer.valueOf(x);
          exit: {
            try {
              if (x < 0) break exit;
              observed = 10;
            } finally {
              observed += 20;
            }
          }
          return value;
        }
        public static String castAfterCleanup(int x) {
          Object value = x == 0 ? "value" : new Object();
          exit: {
            try {
              if (x < 0) break exit;
              observed = 10;
            } finally {
              observed += 20;
            }
          }
          return (String)value;
        }
      }
      """);
    assertFalse(source.contains("Couldn't be decompiled"), source);
    if (!deinline) assertTrue(source.contains("Semaphore variable"), source);
    recompile();
    try (URLClassLoader original = loader(outRoot());
         URLClassLoader recompiled = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      for (String method : List.of("overwrite", "conditional", "nested", "loop", "returnBeforeCleanup", "returnAfterCleanup",
                                  "throwAfterCleanup", "unboxAfterCleanup", "castAfterCleanup", "clearResource",
                                  "conditionalDefinition", "returnOutsideCatch", "throwOutsideCatch")) {
        for (int x : new int[]{Integer.MIN_VALUE, -1, 0, 1, 2, 3, Integer.MAX_VALUE}) {
          assertEquals(invoke(original, method, x), invoke(recompiled, method, x), method + "(" + x + ")");
        }
      }
    }
  }

  private static URLClassLoader loader(Path classes) throws Exception {
    return new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader());
  }

  private static List<Object> invoke(ClassLoader loader, String method, int x) throws Exception {
    Class<?> type = loader.loadClass("pkg.FinallyValues");
    var observed = type.getField("observed");
    observed.setInt(null, 0);
    Object result;
    try {
      result = type.getMethod(method, int.class).invoke(null, x);
    } catch (InvocationTargetException exception) {
      result = exception.getCause().getClass();
    }
    return List.of(result, observed.getInt(null));
  }
}
