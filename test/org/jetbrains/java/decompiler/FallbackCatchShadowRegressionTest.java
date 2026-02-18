package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class FallbackCatchShadowRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testFallbackCatchRewriteDoesNotShadowLaterRuntimeCatch() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestFallbackCatchDoesNotShadowLater.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestFallbackCatchDoesNotShadowLater.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);

    int firstRuntimeCatch = content.indexOf("catch (RuntimeException");
    assertTrue(firstRuntimeCatch >= 0, content);
    int secondRuntimeCatch = content.indexOf("catch (RuntimeException", firstRuntimeCatch + 1);
    assertTrue(secondRuntimeCatch < 0, content);

    Path decompiledFile = fixture.getTargetDir().resolve("pkg/TestFallbackCatchDoesNotShadowLater.java");
    if (!Files.isRegularFile(decompiledFile)) {
      decompiledFile = fixture.getTargetDir().resolve("TestFallbackCatchDoesNotShadowLater.java");
    }
    compileJava8(decompiledFile, fixture.getTempDir().resolve("compile-out"));
  }
}
