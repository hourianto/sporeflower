package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class FinallySourceRecoveryTest extends DecompileRegressionTestBase {
  @Test
  public void testNonThrowingRangeDoesNotDuplicateFinallyCleanup() throws IOException {
    String content = decompileFixture("TestDuplicateFinallyHandlerRange");
    assertEquals(1, countOccurrences(content, "cleanup();"), content);

    recompile();
  }

  @Test
  public void testFallbackFinallySemaphoreGuardIsNotDead() throws IOException {
    String content = decompileFixture("TestFinallySemaphoreFallbackGuard");
    assertContains(content, "$VF: Could not verify finally blocks");

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

  @Test
  public void testSparseProtectedLoopFinallyRangesDecompile() throws IOException {
    String content = decompileFixture("TestSparseProtectedRangeFinally");
    assertMethodDecompiled(content, "DataInputStream\\s+findStream\\s*\\(");
    assertMethodDecompiled(content, "int\\s+findSlot\\s*\\(");
    assertNotContains(content, "Statement cannot be decomposed although reducible");
    recompile();
  }

  private void assertMethodDecompiled(String content, String methodPattern) {
    // The shared decompiler helper rejects error stubs. Also require the
    // declaration itself, so silently dropping the method cannot pass.
    assertTrue(Pattern.compile(methodPattern).matcher(content).find(), "Missing method: " + methodPattern + "\n" + content);
  }
}
