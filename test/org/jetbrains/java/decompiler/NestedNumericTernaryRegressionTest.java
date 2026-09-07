package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;

public class NestedNumericTernaryRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void nestedDefinitionKeepsItsWideNumericType() throws Exception {
    String source = compileDecompileAndRead("pkg/TestNestedNumericTernary.java", """
      package pkg;
      public class TestNestedNumericTernary {
        public static long scale(String text) {
          long value;
          return ((value = Long.parseLong(text)) == 0L ? 1L : value) * 24L * 3600L * 1000L;
        }
      }
      """);
    assertFalse(source.contains("Couldn't be decompiled"), source);
    recompile();
    for (Path classes : new Path[]{outRoot(), fixture.getTempDir().resolve("recompiled-out")}) {
      try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
        var scale = loader.loadClass("pkg.TestNestedNumericTernary").getMethod("scale", String.class);
        for (long value : new long[]{0, 1, -1, Integer.MAX_VALUE + 1L, Long.MAX_VALUE}) {
          assertEquals((value == 0 ? 1 : value) * 24L * 3600L * 1000L, scale.invoke(null, Long.toString(value)));
        }
      }
    }
  }
}
