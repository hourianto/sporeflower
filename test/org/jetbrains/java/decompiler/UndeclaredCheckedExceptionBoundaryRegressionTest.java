package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class UndeclaredCheckedExceptionBoundaryRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testUndeclaredCheckedExceptionIsWrappedAtNoThrowsOverrideBoundary() throws IOException {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path baseClass = jasmClasses.resolve("TestUndeclaredCheckedExceptionOverrideBoundaryBase.class");
    Path testClass = jasmClasses.resolve("TestUndeclaredCheckedExceptionOverrideBoundary.class");
    assertTrue(Files.isRegularFile(baseClass), "Missing test class: " + baseClass);
    assertTrue(Files.isRegularFile(testClass), "Missing test class: " + testClass);

    Path input = fixture.getTempDir().resolve("undeclared-boundary/pkg");
    Files.createDirectories(input);
    Files.copy(baseClass, input.resolve(baseClass.getFileName()));
    Files.copy(testClass, input.resolve(testClass.getFileName()));

    String content = decompileDirectory(input.getParent(), "pkg/TestUndeclaredCheckedExceptionOverrideBoundary.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("helper() throws IOException"), content);
    assertTrue(content.contains("public void run() {"), content);
    assertFalse(content.contains("public void run() throws IOException"), content);
    assertTrue(content.contains("catch (IOException"), content);
    assertTrue(content.contains("new RuntimeException"), content);

    recompile();
  }
}
