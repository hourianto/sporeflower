package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class PartialMultiCatchRewriteRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testUnthrowableCheckedTypeIsKeptWithoutDiscardingValidMultiCatchTypes() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestPartialMultiCatchRewrite.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestPartialMultiCatchRewrite.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);

    // IOException is kept with a reachability marker, and ClassNotFoundException
    // stays as a naturally reachable catch type.
    assertTrue(content.contains("IOException"), content);
    assertTrue(content.contains("ClassNotFoundException"), content);
    assertFalse(content.contains("catch (RuntimeException"), content);

    Path decompiledFile = fixture.getTargetDir().resolve("pkg/TestPartialMultiCatchRewrite.java");
    if (!Files.isRegularFile(decompiledFile)) {
      decompiledFile = fixture.getTargetDir().resolve("TestPartialMultiCatchRewrite.java");
    }
    compileJava8(decompiledFile, fixture.getTempDir().resolve("compile-out"));
  }
}
