package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;

class SharedSwitchFallbackRegressionTest extends DecompileRegressionTestBase {
  @Test
  void switchDefaultRetainsSharedAssignment() throws Exception {
    assertConversionBehavior(fixture.getTestDataDir().resolve("classes/jasm"));
    String source = decompileClassFile(
      fixture.getTestDataDir().resolve("classes/jasm/pkg/TestSharedSwitchFallback.class"),
      "pkg/TestSharedSwitchFallback.java");
    assertFalse(source.contains("$VF: Couldn't be decompiled"), source);
    recompile();
    assertConversionBehavior(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void assertConversionBehavior(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
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
