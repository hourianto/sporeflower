package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class ExternalCheckedCatchWithoutThrowsRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[] {
      IFernflowerPreferences.RENAME_ENTITIES, "1",
      IFernflowerPreferences.LEGACY_SOURCE_COMPATIBILITY, "1"
    };
  }

  @Test
  public void testExternalCheckedCatchWithoutDeclaredThrowsCompiles() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestExternalCheckedCatchWithoutThrows.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    Path exceptionStub = writeSource("ext/ExternalCheckedException.java", """
      package ext;

      public class ExternalCheckedException extends Exception {
      }
      """);
    Path apiStub = writeSource("ext/ExternalApi.java", """
      package ext;

      public class ExternalApi {
        public void notifyDone() {
        }
      }
      """);
    Path libraryOut = fixture.getTempDir().resolve("external-lib");
    compileJava8(List.of(exceptionStub, apiStub), libraryOut);
    fixture.getDecompiler().addLibrary(libraryOut.toFile());

    String content = decompileClassFile(classFile, "pkg/TestExternalCheckedCatchWithoutThrows.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("catch (ExternalCheckedException"), content);
    assertFalse(content.contains("catch (RuntimeException"), content);

    recompile(List.of(exceptionStub, apiStub));
  }
}
