package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;

public class ConsecutiveReadsRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void preservesIndependentCallsAndActualStackCopies() throws Exception {
    Path original = fixture.getTestDataDir().resolve("classes/jasm");
    checkBehavior(original);
    decompileClassFile(original.resolve("pkg/TestConsecutiveReads.class"), "pkg/TestConsecutiveReads.java");
    recompile();
    checkBehavior(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void checkBehavior(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("pkg.TestConsecutiveReads");
      for (String method : new String[]{"distinct", "shared"}) {
        int reads = method.equals("distinct") ? 2 : 1;
        for (int seed : new int[]{0, -1, -2}) {
          type.getField("calls").setInt(null, seed);
          int first = seed + 1;
          int expected = first * 3 + (first > 0 ? 12 : 0) + seed + reads;
          String context = classes + ": " + method + "(" + seed + ")";
          assertEquals(expected, type.getMethod(method).invoke(null), context);
          assertEquals(seed + reads, type.getField("calls").getInt(null), context + " call count");
        }
      }
    }
  }
}
