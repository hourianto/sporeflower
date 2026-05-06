package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class FinallySemaphoreFallbackGuardRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testFallbackFinallySemaphoreGuardIsNotDead() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestFinallySemaphoreFallbackGuard.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestFinallySemaphoreFallbackGuard.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("$VF: Could not verify finally blocks"), content);

    Matcher semaphore = Pattern.compile("boolean (var\\d+) = false /\\* VF: Semaphore variable \\*/;").matcher(content);
    assertTrue(semaphore.find(), content);
    String semaphoreName = semaphore.group(1);

    assertTrue(
      Pattern.compile("\\b" + Pattern.quote(semaphoreName) + "\\s*=\\s*true\\b").matcher(content).find(),
      "Fallback finally semaphore is never enabled:\n" + content
    );
    assertTrue(
      Pattern.compile("\\b" + Pattern.quote(semaphoreName) + "\\s*=\\s*false\\s*;").matcher(content).find(),
      "Fallback finally semaphore exits are not written back to the guard local:\n" + content
    );
    assertFalse(
      Pattern.compile("boolean (?!" + Pattern.quote(semaphoreName) + "\\b)var\\d+ = true\\b").matcher(content).find(),
      "Fallback finally semaphore was split into a different true-initialized local:\n" + content
    );

    recompile();
  }
}
