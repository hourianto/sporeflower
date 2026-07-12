package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertTrue;

public class LegacyPreciseRethrowCompatibilityRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[] {IFernflowerPreferences.LEGACY_SOURCE_COMPATIBILITY, "1"};
  }

  @Test
  public void testLegacySourceDeclaresBroadCatchRethrowType() throws IOException {
    Path source = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestLegacyBroadCatchRethrow.class");
    assertTrue(Files.isRegularFile(source), "Missing test class: " + source);
    String content = decompileClassFile(source, "pkg/TestLegacyBroadCatchRethrow.java");

    assertTrue(content.contains("throws IOException, Exception"), content);
    recompile();
  }
}
