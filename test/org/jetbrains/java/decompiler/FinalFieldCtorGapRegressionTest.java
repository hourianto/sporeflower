package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class FinalFieldCtorGapRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testFinalFieldWithoutAssignmentInAllConstructorsCompiles() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestFinalFieldCtorGap.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestFinalFieldCtorGap.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);

    Path decompiledFile = fixture.getTargetDir().resolve("pkg/TestFinalFieldCtorGap.java");
    if (!Files.isRegularFile(decompiledFile)) {
      decompiledFile = fixture.getTargetDir().resolve("TestFinalFieldCtorGap.java");
    }

    compileJava8(decompiledFile, fixture.getTempDir().resolve("compile-out"));
  }

  @Test
  public void testStaticFinalFieldWithoutInitializerCompiles() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestStaticFinalFieldNoInitializer.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestStaticFinalFieldNoInitializer.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(content.contains("static final int G"), content);
    assertTrue(content.contains("static int G"), content);

    Path decompiledFile = fixture.getTargetDir().resolve("pkg/TestStaticFinalFieldNoInitializer.java");
    if (!Files.isRegularFile(decompiledFile)) {
      decompiledFile = fixture.getTargetDir().resolve("TestStaticFinalFieldNoInitializer.java");
    }

    compileJava8(decompiledFile, fixture.getTempDir().resolve("compile-out-static"));
  }

  @Test
  public void testStaticFinalFieldWithoutDefiniteInitializerCompiles() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestStaticFinalFieldPartialInitializer.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestStaticFinalFieldPartialInitializer.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(content.contains("static final int G"), content);
    assertTrue(content.contains("static int G"), content);

    Path decompiledFile = fixture.getTargetDir().resolve("pkg/TestStaticFinalFieldPartialInitializer.java");
    if (!Files.isRegularFile(decompiledFile)) {
      decompiledFile = fixture.getTargetDir().resolve("TestStaticFinalFieldPartialInitializer.java");
    }

    compileJava8(decompiledFile, fixture.getTempDir().resolve("compile-out-static-partial"));
  }
}
