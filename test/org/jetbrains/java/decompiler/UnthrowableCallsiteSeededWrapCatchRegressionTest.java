package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * An override chain where the hierarchy root declares a checked exception, the
 * direct superclass override does not, and a same-class call site still catches
 * the checked type. The callsite-seeded inference must not synthesize a wrap
 * catch for an exception the rendered body cannot visibly throw: javac rejects
 * such a catch as unreachable.
 */
public class UnthrowableCallsiteSeededWrapCatchRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[] {
      IFernflowerPreferences.RENAME_ENTITIES, "1",
      IFernflowerPreferences.LEGACY_SOURCE_COMPATIBILITY, "1"
    };
  }

  @Test
  public void testCallsiteSeededCheckedExceptionDoesNotEmitUnreachableWrapCatch() throws IOException {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path baseClass = jasmClasses.resolve("TestCallsiteSeededWrapCatchBase.class");
    Path childClass = jasmClasses.resolve("TestCallsiteSeededWrapCatchChild.class");
    assertTrue(Files.isRegularFile(baseClass), "Missing test class: " + baseClass);
    assertTrue(Files.isRegularFile(childClass), "Missing test class: " + childClass);

    Path exceptionStub = writeSource("ext/DeviceStateException.java", """
      package ext;

      public class DeviceStateException extends Exception {
      }
      """);
    Path deviceStub = writeSource("ext/AbstractDevice.java", """
      package ext;

      public abstract class AbstractDevice {
        protected abstract void start() throws DeviceStateException;
      }
      """);
    Path libraryOut = fixture.getTempDir().resolve("device-lib");
    compileJava8(List.of(exceptionStub, deviceStub), libraryOut);
    fixture.getDecompiler().addLibrary(libraryOut.toFile());

    Path input = fixture.getTempDir().resolve("wrap-catch-input/pkg");
    Files.createDirectories(input);
    Files.copy(baseClass, input.resolve("TestCallsiteSeededWrapCatchBase.class"));
    Files.copy(childClass, input.resolve("TestCallsiteSeededWrapCatchChild.class"));

    String content = decompileDirectory(input.getParent(), "pkg/TestCallsiteSeededWrapCatchChild.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(content.contains("$VF$ex"), "Unreachable wrap catch was synthesized:\n" + content);

    recompile(List.of(exceptionStub, deviceStub));
  }
}
