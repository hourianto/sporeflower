package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class FloatDupStoreTernaryRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testFloatTernaryWithDupStoreStackMergeDoesNotFallBack() throws Exception {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestFloatDupStoreTernary.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestFloatDupStoreTernary.java");

    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(content.contains("No common supertype for ternary expression"), content);
    assertTrue(content.contains("public static float min4(float"), content);
    assertTrue(content.contains("public static float max4(float"), content);

    Path decompiledFile = fixture.getTargetDir().resolve("pkg/TestFloatDupStoreTernary.java");
    if (!Files.isRegularFile(decompiledFile)) {
      decompiledFile = fixture.getTargetDir().resolve("TestFloatDupStoreTernary.java");
    }

    Path rebuilt = fixture.getTempDir().resolve("compile-out");
    compileJava8(decompiledFile, rebuilt);
    // Local names are free to coalesce. Verify the duplicated values survive
    // the nested assignments, including unordered comparisons and signed zero.
    try (URLClassLoader original = new URLClassLoader(new URL[]{fixture.getTestDataDir().resolve("classes/jasm").toUri().toURL()},
           ClassLoader.getPlatformClassLoader());
         URLClassLoader recompiled = new URLClassLoader(new URL[]{rebuilt.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      float[] values = {-Float.MAX_VALUE, -1, -0.0F, 0.0F, 1, Float.MAX_VALUE, Float.NaN, Float.POSITIVE_INFINITY, Float.NEGATIVE_INFINITY};
      for (String name : new String[]{"min4", "max4"}) {
        var before = original.loadClass("pkg.TestFloatDupStoreTernary").getMethod(name, float.class, float.class, float.class, float.class);
        var after = recompiled.loadClass("pkg.TestFloatDupStoreTernary").getMethod(name, float.class, float.class, float.class, float.class);
        for (float a : values) for (float b : values) for (float c : values) for (float d : values) {
          int expected = Float.floatToIntBits((float)before.invoke(null, a, b, c, d));
          int actual = Float.floatToIntBits((float)after.invoke(null, a, b, c, d));
          assertEquals(expected, actual, () -> name + "(" + a + ", " + b + ", " + c + ", " + d + ")\n" + content);
        }
      }
    }
  }
}
