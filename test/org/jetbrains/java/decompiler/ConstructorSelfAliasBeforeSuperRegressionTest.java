package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class ConstructorSelfAliasBeforeSuperRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testSelfAliasBeforeSuperDoesNotRenderBeforeConstructorCall() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestConstructorSelfAliasBeforeSuper.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestConstructorSelfAliasBeforeSuper.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(content.contains("var1 = this"), content);
    assertTrue(content.contains("this.value = \"ready\""), content);

    recompile();
  }
}
