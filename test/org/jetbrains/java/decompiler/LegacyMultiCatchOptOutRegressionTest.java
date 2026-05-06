package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertTrue;

public class LegacyMultiCatchOptOutRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[] {IFernflowerPreferences.LEGACY_SOURCE_COMPATIBILITY, "0"};
  }

  @Test
  public void testPreJava7SharedHandlerCanStillRenderMultiCatchWhenOptedOut() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestLegacyMultiCatchRendering.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestLegacyMultiCatchRendering.java");

    assertTrue(content.contains("catch (IOException | ClassNotFoundException"), content);
  }
}
