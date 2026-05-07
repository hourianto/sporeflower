package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class FloatDupStoreTernaryRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testFloatTernaryWithDupStoreStackMergeDoesNotFallBack() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestFloatDupStoreTernary.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestFloatDupStoreTernary.java");

    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(content.contains("No common supertype for ternary expression"), content);
    assertTrue(content.contains("public static float min4(float"), content);
    assertTrue(content.contains("public static float max4(float"), content);
    assertTrue(content.contains("? var5 : var3"), content);

    Path decompiledFile = fixture.getTargetDir().resolve("pkg/TestFloatDupStoreTernary.java");
    if (!Files.isRegularFile(decompiledFile)) {
      decompiledFile = fixture.getTargetDir().resolve("TestFloatDupStoreTernary.java");
    }

    compileJava8(decompiledFile, fixture.getTempDir().resolve("compile-out"));
  }
}
