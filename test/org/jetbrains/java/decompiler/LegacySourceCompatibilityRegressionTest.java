package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class LegacySourceCompatibilityRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testPreJava5ArrayLoopDoesNotRenderEnhancedForByDefault() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestLegacyArrayForeachRendering.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestLegacyArrayForeachRendering.java");

    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(content.contains(" : "), content);
    assertTrue(content.contains("for (int "), content);
  }
}
