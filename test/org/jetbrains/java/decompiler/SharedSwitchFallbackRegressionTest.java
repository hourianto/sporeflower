package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.net.URLClassLoader;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;

class SharedSwitchFallbackRegressionTest extends DecompileRegressionTestBase {
  @Test
  void switchDefaultRetainsSharedAssignment() throws Exception {
    assertConversionBehavior(fixture.getTestDataDir().resolve("classes/jasm"));
    String source = decompileClassFile(
      fixture.getTestDataDir().resolve("classes/jasm/pkg/TestSharedSwitchFallback.class"),
      "pkg/TestSharedSwitchFallback.java");
    recompile();
    assertConversionBehavior(recompiledRoot());
  }

  private static void assertConversionBehavior(Path classes) throws Exception {
    try (URLClassLoader loader = loader(classes)) {
      var convert = loader.loadClass("pkg.TestSharedSwitchFallback").getMethod("convert", int.class);
      for (int value = -5; value <= 50; value++) {
        int expected = value >= 10 && value < 20 ? value + 1 : value >= 20 && value < 30 ? value + 2 : value == 40 ? 99 : value;
        assertEquals(expected, convert.invoke(null, value), "Input " + value);
      }
      assertEquals(Integer.MIN_VALUE, convert.invoke(null, Integer.MIN_VALUE));
      assertEquals(Integer.MAX_VALUE, convert.invoke(null, Integer.MAX_VALUE));
    }
  }
}
