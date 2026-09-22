package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

public class DeadNullLocalRegressionTest extends DecompileRegressionTestBase {
  @Test
  void removesDeadNullStoresBeforeReferenceReuseAcrossBranches() throws Exception {
    Path source = writeSource("NullLocals.java", """
      public class NullLocals {
        public static String dead(boolean choose, String input) {
          Object unused = null;
          if (choose) unused = input.trim();
          else unused = input.toUpperCase();
          return unused.toString();
        }

        public static String unrelated(String input) {
          { Object unused = null; }
          { String value = input.trim(); return value + value; }
        }

        public static String live(boolean choose, String input) {
          String value = null;
          if (choose) value = input.trim();
          return value;
        }

        public static String handler(boolean fail) {
          String value = null;
          try {
            if (fail) throw new IllegalArgumentException();
            value = "done";
          } catch (IllegalArgumentException exception) {
            return value;
          }
          return value;
        }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    String output = decompileDirectory(outRoot(), "NullLocals.java");
    recompile();
    for (Path classes : List.of(outRoot(), fixture.getTempDir().resolve("recompiled-out"))) {
      try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
        Class<?> type = loader.loadClass("NullLocals");
        assertEquals(" text ".toUpperCase(), type.getMethod("dead", boolean.class, String.class).invoke(null, false, " text "));
        assertEquals("text", type.getMethod("dead", boolean.class, String.class).invoke(null, true, " text "));
        assertEquals("texttext", type.getMethod("unrelated", String.class).invoke(null, " text "));
        assertNull(type.getMethod("live", boolean.class, String.class).invoke(null, false, " text "));
        assertEquals("text", type.getMethod("live", boolean.class, String.class).invoke(null, true, " text "));
        assertNull(type.getMethod("handler", boolean.class).invoke(null, true));
        assertEquals("done", type.getMethod("handler", boolean.class).invoke(null, false));
      }
    }
    String deadMethods = output.substring(output.indexOf("public static String dead"), output.indexOf("public static String live"));
    assertFalse(deadMethods.contains("= null"), output);
  }
}
