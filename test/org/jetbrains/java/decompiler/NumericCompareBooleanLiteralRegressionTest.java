package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class NumericCompareBooleanLiteralRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testByteComparisonDoesNotRenderBooleanLiteral() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestNumericCompareBooleanLiteral.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestNumericCompareBooleanLiteral.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(content.contains("!= true"), content);
    assertFalse(content.contains("== true"), content);

    Path decompiledFile = fixture.getTargetDir().resolve("pkg/TestNumericCompareBooleanLiteral.java");
    if (!Files.isRegularFile(decompiledFile)) {
      decompiledFile = fixture.getTargetDir().resolve("TestNumericCompareBooleanLiteral.java");
    }

    compileJava8(decompiledFile, fixture.getTempDir().resolve("compile-out"));
  }
}
