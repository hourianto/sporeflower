package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.nio.file.Path;

public class ConsecutiveReadsRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void preservesIndependentCallsAndActualStackCopies() throws Exception {
    Path original = fixture.getTestDataDir().resolve("classes/jasm");
    decompileClassFile(original.resolve("pkg/TestConsecutiveReads.class"), "pkg/TestConsecutiveReads.java");
    recompile();
    try (BehaviorComparison behavior = compareBehavior(original)) {
      for (String method : new String[]{"distinct", "shared"}) {
        int reads = method.equals("distinct") ? 2 : 1;
        for (int seed : new int[]{0, -1, -2}) {
          int first = seed + 1;
          int expected = first * 3 + (first > 0 ? 12 : 0) + seed + reads;
          behavior.assertSame(method + "(" + seed + ")", new Object[]{expected, seed + reads}, loader -> {
            Class<?> type = loader.loadClass("pkg.TestConsecutiveReads");
            type.getField("calls").setInt(null, seed);
            Object result = type.getMethod(method).invoke(null);
            return new Object[]{result, type.getField("calls").getInt(null)};
          });
        }
      }
    }
  }
}
