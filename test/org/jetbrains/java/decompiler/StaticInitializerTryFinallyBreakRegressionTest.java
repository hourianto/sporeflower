package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class StaticInitializerTryFinallyBreakRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[] {
      IFernflowerPreferences.RENAME_ENTITIES, "1",
      IFernflowerPreferences.LEGACY_SOURCE_COMPATIBILITY, "1"
    };
  }

  @Test
  public void testStaticInitializerBreakThroughFinallyStaysInsideInitializer() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestStaticInitializerTryFinallyBreak.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestStaticInitializerTryFinallyBreak.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(content.contains("return;"), content);

    recompile();
  }
}
