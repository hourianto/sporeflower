package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class OuterInterruptedCatchAfterSwallowedSleepRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[] {
      IFernflowerPreferences.RENAME_ENTITIES, "1",
      IFernflowerPreferences.LEGACY_SOURCE_COMPATIBILITY, "1"
    };
  }

  @Test
  public void testOuterInterruptedCatchAfterSwallowedSleepCompiles() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestOuterInterruptedCatchAfterSwallowedSleep.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    Path legacyCanvas = writeSource("ext/LegacyCanvas.java", """
      package ext;

      public class LegacyCanvas {
        public void repaint() {
        }
      }
      """);
    Path libraryOut = fixture.getTempDir().resolve("legacy-ui-lib");
    compileJava8(legacyCanvas, libraryOut);
    fixture.getDecompiler().addLibrary(libraryOut.toFile());

    String content = decompileClassFile(classFile, "pkg/TestOuterInterruptedCatchAfterSwallowedSleep.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("catch (InterruptedException"), content);
    assertFalse(content.contains("catch (RuntimeException"), content);

    recompile(List.of(legacyCanvas));
  }
}
