package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.net.URL;
import java.net.URLClassLoader;
import java.util.Arrays;
import java.util.List;
import java.nio.file.Path;
import java.time.Duration;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTimeout;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class ByteArrayMergeRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testByteArrayVariableKeepsArrayTypeAfterNullInitialization() throws Exception {
    String content = compileDecompileAndRead("pkg/TestByteArrayMerge.java", """
package pkg;

public class TestByteArrayMerge {
  private static byte[] decode(int size, int step) {
    byte[] pixels = null;
    if (size > 0) {
      pixels = new byte[size];
    } else {
      pixels = new byte[1];
    }

    int value = step & 255;
    for (int i = 0; i < pixels.length; i++) {
      pixels[i] = (byte)value;
    }

    return pixels;
  }
}
""");

    assertTrue(content.contains("private static byte[] decode"), content);
    assertTrue(Pattern.compile("\\bbyte\\[]\\s+var\\d+;").matcher(content).find(), content);
    assertFalse(Pattern.compile("\\bB\\s+var\\d+\\s*=\\s*null;").matcher(content).find(), content);
    assertFalse(content.contains("(Object[])"), content);
    recompile();
    for (Path classes : List.of(outRoot(), fixture.getTempDir().resolve("recompiled-out"))) {
      try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
        var decode = loader.loadClass("pkg.TestByteArrayMerge").getDeclaredMethod("decode", int.class, int.class);
        decode.setAccessible(true);
        for (int size : new int[]{-1, 0, 1, 17}) {
          for (int step : new int[]{Integer.MIN_VALUE, -1, 0, 127, 128, 255, Integer.MAX_VALUE}) {
            byte[] expected = new byte[Math.max(size, 1)];
            Arrays.fill(expected, (byte)step);
            assertArrayEquals(expected, (byte[])decode.invoke(null, size, step));
          }
        }
      }
    }
  }

  @Test
  public void testLegacyStackMapObjectFrameKeepsPrimitiveArrayMergeAtObject() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestPrimitiveArrayObjectMergeStackMap.class");
    String content = assertTimeout(Duration.ofSeconds(10), () ->
      decompileClassFile(classFile, "pkg/TestPrimitiveArrayObjectMergeStackMap.java"));

    assertTrue(content.contains("public static Object decode"), content);
    assertFalse(Pattern.compile("\\bObject\\[]\\s+var\\d+;").matcher(content).find(), content);

    assertTimeout(Duration.ofSeconds(10), () -> {
      recompile();
    });
  }
}
