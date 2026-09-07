package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.lang.reflect.InvocationTargetException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class FinallyReturnRangeRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void splitReturnDoesNotLeaveAGapInTheOuterCatch() throws Exception {
    checkRoundTrip("TestFinallyReturnRange");
  }

  @Test
  public void distinctCleanupPathsKeepTheirContinuations() throws Exception {
    checkRoundTrip("TestFinallyDistinctContinuation");
  }

  @Test
  public void cleanupCatchDoesNotTurnSuccessIntoFailure() throws Exception {
    checkRoundTrip("TestFinallyCleanupCatch");
  }

  private void checkRoundTrip(String className) throws Exception {
    Path state = writeSource("pkg/FinallyReturnState.java", """
      package pkg;
      public class FinallyReturnState {
        public static Object stream;
        public static boolean connection, success, error;
        public static int failAt, cleanupFailAt;
        public static String trace;
        public static Object open() {
          step(1);
          return connection ? new Object() : null;
        }
        public static void step(int id) {
          trace += id;
          if (id == failAt || id == cleanupFailAt) {
            if (error) throw new AssertionError("step" + id);
            throw new IllegalStateException("step" + id);
          }
        }
        public static void caught(Exception exception) {
          trace += " caught:" + exception.getMessage();
        }
      }
      """);
    compileJava8(state, outRoot());
    Path originalClasses = fixture.getTestDataDir().resolve("classes/jasm");
    String content = decompileClassFile(originalClasses.resolve("pkg/" + className + ".class"),
      "pkg/" + className + ".java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    if (className.equals("TestFinallyReturnRange")) {
      assertFalse(content.contains("semaphore"), content);
    }
    assertTrue(content.contains("finally"), content);
    recompile(List.of(state));

    try (URLClassLoader original = loader(originalClasses, outRoot());
         URLClassLoader recompiled = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      if (className.equals("TestFinallyReturnRange")) {
        assertEquals("12356", execute(original, className, true, true, true, 0, false));
        assertEquals("1235", execute(original, className, true, false, true, 0, false));
        assertEquals("1256 caught:step2", execute(original, className, true, true, true, 2, false));
        assertEquals("1235 caught:step5", execute(original, className, true, true, true, 5, false));
        assertEquals("1256 throws:java.lang.AssertionError:step2", execute(original, className, true, true, true, 2, true));
      } else if (className.equals("TestFinallyCleanupCatch")) {
        assertEquals("126 result:true", execute(original, className, true, true, true, 0, false));
        assertEquals("126 result:false", execute(original, className, true, true, true, 6, false));
      }
      for (boolean stream : new boolean[]{false, true}) {
        for (boolean connection : new boolean[]{false, true}) {
          for (boolean success : new boolean[]{false, true}) {
            for (int failAt = 0; failAt <= 8; failAt++) {
              for (boolean error : new boolean[]{false, true}) {
                String expected = execute(original, className, stream, connection, success, failAt, error);
                assertEquals(expected, execute(recompiled, className, stream, connection, success, failAt, error),
                  "stream=" + stream + ", connection=" + connection + ", success=" + success
                    + ", failAt=" + failAt + ", error=" + error);
              }
            }
          }
        }
      }
      // A cleanup failure must replace a pending request failure, and an
      // earlier cleanup failure must prevent later cleanup from executing.
      for (int cleanupFailAt : new int[]{5, 6}) {
        original.loadClass("pkg.FinallyReturnState").getField("cleanupFailAt").setInt(null, cleanupFailAt);
        recompiled.loadClass("pkg.FinallyReturnState").getField("cleanupFailAt").setInt(null, cleanupFailAt);
        for (int failAt : new int[]{1, 2, 5}) {
          for (boolean error : new boolean[]{false, true}) {
            assertEquals(execute(original, className, true, true, true, failAt, error),
              execute(recompiled, className, true, true, true, failAt, error));
          }
        }
      }
    }
  }

  private static URLClassLoader loader(Path... roots) throws Exception {
    URL[] urls = new URL[roots.length];
    for (int i = 0; i < roots.length; i++) {
      urls[i] = roots[i].toUri().toURL();
    }
    return new URLClassLoader(urls, ClassLoader.getPlatformClassLoader());
  }

  private static String execute(ClassLoader loader, String className, boolean stream, boolean connection, boolean success,
                                int failAt, boolean error) throws Exception {
    Class<?> state = loader.loadClass("pkg.FinallyReturnState");
    state.getField("stream").set(null, stream ? new Object() : null);
    state.getField("connection").setBoolean(null, connection);
    state.getField("success").setBoolean(null, success);
    state.getField("failAt").setInt(null, failAt);
    state.getField("error").setBoolean(null, error);
    state.getField("trace").set(null, "");
    String thrown = "";
    try {
      Object result = loader.loadClass("pkg." + className).getMethod("run").invoke(null);
      if (result != null) {
        thrown = " result:" + result;
      }
    } catch (InvocationTargetException exception) {
      Throwable cause = exception.getCause();
      thrown = " throws:" + cause.getClass().getName() + ":" + cause.getMessage();
    }
    return state.getField("trace").get(null) + thrown;
  }
}
