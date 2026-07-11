package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class LegacyTernaryReferenceCastRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testSiblingReferenceTernaryCastsOneBranchForLegacySource() throws IOException {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path input = fixture.getTempDir().resolve("legacy-ternary-input/pkg");
    Files.createDirectories(input);
    Files.copy(jasmClasses.resolve("TestLegacyTernaryReferenceCast.class"), input.resolve("TestLegacyTernaryReferenceCast.class"));
    Files.copy(jasmClasses.resolve("TestLegacyTernaryReferenceCastBase.class"), input.resolve("TestLegacyTernaryReferenceCastBase.class"));
    Files.copy(jasmClasses.resolve("TestLegacyTernaryReferenceCastLeft.class"), input.resolve("TestLegacyTernaryReferenceCastLeft.class"));
    Files.copy(jasmClasses.resolve("TestLegacyTernaryReferenceCastRight.class"), input.resolve("TestLegacyTernaryReferenceCastRight.class"));

    String content = decompileDirectory(input.getParent(), "pkg/TestLegacyTernaryReferenceCast.java");

    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(
      content.contains("? (TestLegacyTernaryReferenceCastBase)(new TestLegacyTernaryReferenceCastLeft()) : new TestLegacyTernaryReferenceCastRight()"),
      content
    );
    assertTrue(
      content.contains("? (Object)(new TestLegacyTernaryReferenceCastLeft()) : new byte[1]"),
      content
    );
    assertTrue(
      content.contains("? (Object)(new byte[1]) : new TestLegacyTernaryReferenceCastLeft()"),
      content
    );
    assertTrue(
      content.contains("? (Object[])(new String[1]) : new Integer[1]"),
      content
    );
    assertTrue(content.contains("(TestLegacyTernaryReferenceCastBase)var2 != var3"), content);
    assertTrue(content.contains("return var0 == var1;"), content);
    assertTrue(content.contains("return (Object[])var0 != var1;"), content);
    assertTrue(content.contains("return (Object)var0 != var1;"), content);

    recompile();
  }
}
