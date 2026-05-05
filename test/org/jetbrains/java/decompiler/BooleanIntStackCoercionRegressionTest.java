package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class BooleanIntStackCoercionRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testBooleanAndIntegerStackCoercionsRenderAsJavaConversions() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestBooleanIntStackCoercion.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestBooleanIntStackCoercion.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(content.contains("(short)flag()"), content);
    assertFalse(content.contains("(boolean)this.p"), content);
    assertFalse(content.contains("(short)var1"), content);
    assertFalse(content.contains("var0 ? 1 : 0"), content);
    assertTrue(content.contains("return (short)(flag() ? 1 : 0);"), content);
    assertTrue(content.contains("return (this.p != 0);"), content);
    assertTrue(content.contains("this.p = (short)(var1 ? 1 : 0);"), content);
    assertTrue(content.contains("return (var1 != 0 ? (this.p != 0) : flag());"), content);

    Path decompiledFile = fixture.getTargetDir().resolve("pkg/TestBooleanIntStackCoercion.java");
    if (!Files.isRegularFile(decompiledFile)) {
      decompiledFile = fixture.getTargetDir().resolve("TestBooleanIntStackCoercion.java");
    }

    compileJava8(decompiledFile, fixture.getTempDir().resolve("compile-out"));
  }
}
