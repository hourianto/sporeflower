package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class DuplicateFinallyHandlerRangeRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testNonThrowingRangeDoesNotDuplicateFinallyCleanup() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestDuplicateFinallyHandlerRange.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestDuplicateFinallyHandlerRange.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertEquals(1, countOccurrences(content, "cleanup();"), content);

    Path decompiledFile = fixture.getTargetDir().resolve("pkg/TestDuplicateFinallyHandlerRange.java");
    if (!Files.isRegularFile(decompiledFile)) {
      decompiledFile = fixture.getTargetDir().resolve("TestDuplicateFinallyHandlerRange.java");
    }
    compileJava8(decompiledFile, fixture.getTempDir().resolve("compile-out"));
  }

  private static int countOccurrences(String text, String needle) {
    int count = 0;
    for (int index = 0; (index = text.indexOf(needle, index)) >= 0; index += needle.length()) {
      count++;
    }
    return count;
  }
}
