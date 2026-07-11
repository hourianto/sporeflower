package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class CatchHandlersLoopBackRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testCatchHandlersContinuingProtectedLoopRemainInsideCatch() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestCatchHandlersLoopBack.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestCatchHandlersLoopBack.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("catch (InterruptedException"), content);
    assertTrue(content.contains("catch (Exception"), content);

    Path decompiledFile = fixture.getTargetDir().resolve("pkg/TestCatchHandlersLoopBack.java");
    if (!Files.isRegularFile(decompiledFile)) {
      decompiledFile = fixture.getTargetDir().resolve("TestCatchHandlersLoopBack.java");
    }
    compileJava8(decompiledFile, fixture.getTempDir().resolve("compile-out"));
  }
}
