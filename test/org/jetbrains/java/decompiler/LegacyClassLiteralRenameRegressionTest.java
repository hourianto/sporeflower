package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;

public class LegacyClassLiteralRenameRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[] {
      IFernflowerPreferences.RENAME_ENTITIES, "1",
      IFernflowerPreferences.LEGACY_SOURCE_COMPATIBILITY, "1"
    };
  }

  @Test
  public void testLegacyClassLiteralUsesRenamedClassWithoutGenerics() throws IOException {
    decompileFixtureInput();
    assertNotContains(readAllDecompiled(), "Class<", "x.class");
    recompile();
  }

  @Test
  public void testModernClassLiteralUsesRenamedClassWithGenerics() throws IOException {
    configureOptions(IFernflowerPreferences.LEGACY_SOURCE_COMPATIBILITY, "0");
    decompileFixtureInput();
    String content = readAllDecompiled();
    assertNotContains(content, "Class<x>", "x.class");
    assertContains(content, "Class<");
    recompile();
  }

  private void decompileFixtureInput() throws IOException {
    fixture.getDecompiler().addSource(copyJasmClasses("x", "y").toFile());
    fixture.getDecompiler().decompileContext();
  }
}
