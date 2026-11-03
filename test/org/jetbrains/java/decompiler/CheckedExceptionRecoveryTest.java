package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertTrue;

public class CheckedExceptionRecoveryTest extends DecompileRegressionTestBase {
  @Test
  public void testFallbackCatchRewriteDoesNotShadowLaterRuntimeCatch() throws IOException {
    String content = decompileFixture("TestFallbackCatchDoesNotShadowLater");

    int firstRuntimeCatch = content.indexOf("catch (RuntimeException");
    assertTrue(firstRuntimeCatch >= 0, content);
    int secondRuntimeCatch = content.indexOf("catch (RuntimeException", firstRuntimeCatch + 1);
    assertTrue(secondRuntimeCatch < 0, content);

    recompile();
  }

  @Test
  public void testCatchExceptionIsKeptWhenInvocationThrowsViaSuperInterface() throws IOException {
    String content = compileDecompileAndRead("pkg/TestInheritedInterfaceThrowsCatch.java", """
      package pkg;

      import java.io.IOException;

      interface A {
        void close() throws IOException;
      }

      interface B extends A {
      }

      public class TestInheritedInterfaceThrowsCatch {
        B value;

        public void probe() {
          try {
            value.close();
          } catch (Exception e) {
          }
        }
      }
      """);

    assertContains(content, "catch (Exception");
    assertNotContains(content, "substituted checked catch types: Exception");
    recompile();
  }

  @Test
  public void testExternalCheckedCatchWithoutDeclaredThrowsCompiles() throws IOException {
    configureOptions(IFernflowerPreferences.RENAME_ENTITIES, "1");

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

    String content = decompileFixture("TestExternalCheckedCatchWithoutThrows");
    assertContains(content, "catch (ExternalCheckedException");
    assertNotContains(content, "catch (RuntimeException");

    recompile(List.of(exceptionStub, apiStub));
  }

  @Test
  public void testOuterInterruptedCatchAfterSwallowedSleepCompiles() throws IOException {
    configureOptions(IFernflowerPreferences.RENAME_ENTITIES, "1");

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

    String content = decompileFixture("TestOuterInterruptedCatchAfterSwallowedSleep");
    assertContains(content, "catch (InterruptedException");
    assertNotContains(content, "catch (RuntimeException");

    recompile(List.of(legacyCanvas));
  }
}
