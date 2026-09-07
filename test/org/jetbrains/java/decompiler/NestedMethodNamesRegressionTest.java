package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.net.URLClassLoader;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;

class NestedMethodNamesRegressionTest extends DecompileRegressionTestBase {
  @Test
  void siblingMethodsWithTheSameSignatureKeepTheirCapturedValues() throws Exception {
    Path source = writeSource("pkg/NestedMethodNames.java", """
      package pkg;
      public class NestedMethodNames {
        interface Operation { int apply(int value); }
        interface WideOperation { long apply(long value, int extra); }
        public int apply(final int seed) {
          Operation first = new Operation() {
            public int apply(int value) { return seed + value; }
          };
          Operation second = new Operation() {
            public int apply(int value) { return seed * value; }
          };
          WideOperation wide = new WideOperation() {
            public long apply(long value, int extra) { return seed + value + extra; }
          };
          return first.apply(2) + second.apply(3) + (int)wide.apply(7L, 11);
        }
        public static int run() { return new NestedMethodNames().apply(5); }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    assertResult(outRoot());
    decompileDirectory(outRoot(), "pkg/NestedMethodNames.java");
    recompile();
    assertResult(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void assertResult(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new java.net.URL[]{classes.toUri().toURL()}, null)) {
      assertEquals(45, loader.loadClass("pkg.NestedMethodNames").getMethod("run").invoke(null));
    }
  }
}
