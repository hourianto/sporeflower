package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class PrivateSpecialQualifierCastRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testPrivateSpecialCallOnSubtypeQualifierKeepsOwnerCast() throws IOException {
    Path baseClass = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestPrivateSpecialQualifierCastBase.class");
    Path subClass = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestPrivateSpecialQualifierCastSub.class");
    assertTrue(Files.isRegularFile(baseClass), "Missing test class: " + baseClass);
    assertTrue(Files.isRegularFile(subClass), "Missing test class: " + subClass);

    var decompiler = fixture.getDecompiler();
    decompiler.addSource(baseClass.toFile());
    decompiler.addSource(subClass.toFile());
    decompiler.decompileContext();

    Path baseJava = fixture.getTargetDir().resolve("pkg/TestPrivateSpecialQualifierCastBase.java");
    if (!Files.isRegularFile(baseJava)) {
      baseJava = fixture.getTargetDir().resolve("TestPrivateSpecialQualifierCastBase.java");
    }
    Path subJava = fixture.getTargetDir().resolve("pkg/TestPrivateSpecialQualifierCastSub.java");
    if (!Files.isRegularFile(subJava)) {
      subJava = fixture.getTargetDir().resolve("TestPrivateSpecialQualifierCastSub.java");
    }

    String baseContent = DecompilerTestFixture.getContent(baseJava);
    assertFalse(baseContent.contains("$VF: Couldn't be decompiled"), baseContent);
    assertTrue(baseContent.contains("((TestPrivateSpecialQualifierCastBase)"), baseContent);
    assertTrue(baseContent.contains(".secret()"), baseContent);

    compileJava8(List.of(baseJava, subJava), fixture.getTempDir().resolve("compile-out"));
  }
}
