package org.jetbrains.java.decompiler;

import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.CsvSource;
import org.junit.jupiter.params.provider.ValueSource;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.concurrent.TimeUnit;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class FinallyCleanupLoopRegressionTest extends DecompileRegressionTestBase {
  @ParameterizedTest
  @CsvSource({"int,false", "StringBuffer,false", "void,false", "int,true", "StringBuffer,true", "void,true"})
  public void cleanupSwitchPreservesTerminationAndPendingExit(String returnType, boolean returnFromTry) throws Exception {
    String declaration = returnType.equals("void") ? "" : returnType + " result = "
      + (returnType.equals("int") ? "7" : "new StringBuffer(\"ok\")") + ";";
    String returnStatement = returnType.equals("void") ? "" : "return result;";
    String tryBody = returnFromTry ? "if (first.read() >= 0) "
      + (returnType.equals("void") ? "return;" : returnStatement) : "first.read();";
    // Reassign the local during cleanup: an already evaluated return must keep
    // its original value, while a return after the caught IOException sees it.
    String cleanupAssignment = !returnFromTry || returnType.equals("void") ? "" : returnType.equals("int")
      ? "result++;" : "result = new StringBuffer(\"changed\");";
    Path source = writeSource("pkg/CleanupLoop.java", """
      package pkg;
      import java.io.*;
      public class CleanupLoop {
        public static %s run(InputStream first, InputStream second) {
          %s
          try {
            %s
          } catch (IOException ignored) {
          } finally {
            for (int i = 0; i < 2; i++) {
              try {
                switch (i) {
                  case 0:
                    if (first != null) { first.close(); first = null; }
                    break;
                  case 1:
                    if (second != null) { second.close(); second = null; }
                }
              } catch (IOException ignored) {
              }
              %s
            }
          }
          %s
        }
      }
      """.formatted(returnType, declaration, tryBody, cleanupAssignment, returnStatement));
    Path probe = writeSource("pkg/CleanupProbe.java", """
      package pkg;
      import java.io.*;
      import java.lang.reflect.*;
      public class CleanupProbe {
        private static String trace;
        private static Throwable pending;
        private static void fail(int mode, String where) throws IOException {
          switch (mode) {
            case 1: throw new IOException(where);
            case 2: pending = new IllegalStateException(where); break;
            case 3: pending = new AssertionError(where); break;
            default: return;
          }
          if (pending instanceof Error) throw (Error) pending;
          throw (RuntimeException) pending;
        }
        private static class Stream extends InputStream {
          private final int id, readMode, closeMode;
          Stream(int id, int readMode, int closeMode) {
            this.id = id; this.readMode = readMode; this.closeMode = closeMode;
          }
          public int read() throws IOException {
            trace += "r";
            fail(readMode, "read");
            return readMode == 4 ? -1 : 1;
          }
          public void close() throws IOException {
            trace += id;
            fail(closeMode, "close" + id);
          }
        }
        public static void main(String[] args) throws Exception {
          Method method = CleanupLoop.class.getMethod("run", InputStream.class, InputStream.class);
          for (int missing = 0; missing < 4; missing++) {
            for (int read = 0; read < 5; read++) {
              for (int firstClose = 0; firstClose < 4; firstClose++) {
                for (int secondClose = 0; secondClose < 4; secondClose++) {
                  trace = "";
                  pending = null;
                  String result;
                  try {
                    Object value = method.invoke(null,
                      (missing & 1) == 0 ? new Stream(0, read, firstClose) : null,
                      (missing & 2) == 0 ? new Stream(1, read, secondClose) : null);
                    result = "return:" + value;
                  } catch (InvocationTargetException exception) {
                    Throwable cause = exception.getCause();
                    result = "throw:" + cause.getClass().getName();
                    if (!(cause instanceof NullPointerException)) {
                      result += ":" + cause.getMessage() + ":same=" + (cause == pending);
                    }
                  }
                  System.out.println(missing + ":" + read + ":" + firstClose + ":" + secondClose
                    + " " + trace + " " + result);
                }
              }
            }
          }
        }
      }
      """);
    compileJava8NoDebug(List.of(source, probe), outRoot());
    String content = decompileClassFile(outRoot().resolve("pkg/CleanupLoop.class"), "pkg/CleanupLoop.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("finally"), content);
    recompile(List.of(probe));

    String expected = execute(outRoot(), "original");
    String result = returnType.equals("int") ? "7" : returnType.equals("void") ? "null" : "ok";
    assertTrue(expected.contains("0:0:0:0 r01 return:" + result), expected);
    String caughtResult = returnFromTry && !returnType.equals("void") ? returnType.equals("int") ? "9" : "changed" : result;
    assertTrue(expected.contains("0:1:0:0 r01 return:" + caughtResult), expected);
    assertTrue(expected.contains("0:4:0:0 r01 return:" + caughtResult), expected);
    assertTrue(expected.contains("0:2:0:0 r01 throw:java.lang.IllegalStateException:read:same=true"), expected);
    assertTrue(expected.contains("0:3:1:1 r01 throw:java.lang.AssertionError:read:same=true"), expected);
    assertTrue(expected.contains("0:2:3:0 r0 throw:java.lang.AssertionError:close0:same=true"), expected);
    assertEquals(expected, execute(fixture.getTempDir().resolve("recompiled-out"), "recompiled"));
  }

  @ParameterizedTest
  @ValueSource(strings = {"throw new IllegalStateException(\"cleanup\");", "break;", "return;"})
  public void conditionalFinallyExitInsideOuterLoop(String exit) throws Exception {
    Path source = writeSource("pkg/CleanupLoop.java", """
      package pkg;
      public class CleanupLoop {
        public static String trace;
        public static void run(boolean exit, boolean fail) {
          for (int i = 0; i < 3; i++) {
            try {
              trace += "t";
              if (fail) throw new IllegalArgumentException("try");
            } finally {
              trace += "f";
              if (exit) { %s }
            }
          }
        }
      }
      """.formatted(exit));
    Path probe = writeSource("pkg/CleanupProbe.java", """
      package pkg;
      public class CleanupProbe {
        public static void main(String[] args) {
          for (boolean exit : new boolean[]{false, true}) {
            for (boolean fail : new boolean[]{false, true}) {
              CleanupLoop.trace = "";
              String outcome = "return";
              try { CleanupLoop.run(exit, fail); }
              catch (RuntimeException exception) { outcome = exception.toString(); }
              System.out.println(exit + ":" + fail + " " + CleanupLoop.trace + " " + outcome);
            }
          }
        }
      }
      """);
    compileJava8NoDebug(List.of(source, probe), outRoot());
    String content = decompileClassFile(outRoot().resolve("pkg/CleanupLoop.class"), "pkg/CleanupLoop.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    recompile(List.of(probe));
    String expected = execute(outRoot(), "original");
    assertTrue(expected.contains("false:false tftftf return"), expected);
    assertTrue(expected.contains("false:true tf java.lang.IllegalArgumentException: try"), expected);
    String outcome = exit.startsWith("throw") ? "java.lang.IllegalStateException: cleanup" : "return";
    assertTrue(expected.contains("true:false tf " + outcome), expected);
    assertTrue(expected.contains("true:true tf " + outcome), expected);
    assertEquals(expected, execute(fixture.getTempDir().resolve("recompiled-out"), "recompiled"));
  }

  private String execute(Path classes, String name) throws Exception {
    Path output = fixture.getTempDir().resolve(name + ".log");
    String executable = System.getProperty("os.name").startsWith("Windows") ? "java.exe" : "java";
    Process process = new ProcessBuilder(Path.of(System.getProperty("java.home"), "bin", executable).toString(),
      "-cp", classes.toString(), "pkg.CleanupProbe")
      .redirectErrorStream(true).redirectOutput(output.toFile()).start();
    try {
      // The void form can compile despite a lost exit. Run outside the test JVM
      // so a regression that spins without checking interrupts can be stopped.
      assertTrue(process.waitFor(10, TimeUnit.SECONDS), name + " cleanup did not terminate");
      assertEquals(0, process.exitValue(), Files.readString(output));
      return Files.readString(output);
    } finally {
      process.destroyForcibly();
      process.waitFor();
    }
  }
}
