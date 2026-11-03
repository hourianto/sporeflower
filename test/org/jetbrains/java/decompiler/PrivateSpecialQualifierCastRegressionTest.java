package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Path;

public class PrivateSpecialQualifierCastRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testPrivateSpecialCallOnSubtypeQualifierKeepsOwnerCast() throws IOException {
    Path input = copyJasmClasses("TestPrivateSpecialQualifierCastBase", "TestPrivateSpecialQualifierCastSub");
    String content = decompileDirectory(input, "pkg/TestPrivateSpecialQualifierCastBase.java");
    readDecompiled("pkg/TestPrivateSpecialQualifierCastSub.java");
    assertContains(content, "((TestPrivateSpecialQualifierCastBase)", ".secret()");
    recompile();
  }
}
