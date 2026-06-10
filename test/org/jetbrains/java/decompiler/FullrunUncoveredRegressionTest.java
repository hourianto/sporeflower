package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class FullrunUncoveredRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[] {IFernflowerPreferences.RENAME_ENTITIES, "1"};
  }

  @Test
  public void testPackageClassNameCollisionDoesNotQualifySiblingAsNestedMember() throws IOException {
    Path inputRoot = fixture.getTempDir().resolve("package-class-collision-input");
    Path input = inputRoot.resolve("Viewer");
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/Viewer");
    Files.createDirectories(input);
    Files.copy(jasmClasses.resolve("Viewer.class"), input.resolve("Viewer.class"));
    Files.copy(jasmClasses.resolve("jsr75.class"), input.resolve("jsr75.class"));
    Files.copy(fixture.getTestDataDir().resolve("classes/jasm/jsr75.class"), inputRoot.resolve("jsr75.class"));

    String content = decompileDirectory(inputRoot, "Viewer/Viewer.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);

    recompile();
  }

  @Test
  public void testFinalReturnOnlyInterfaceConflictWithFinalSuperclassIsRenamedForSource() throws IOException {
    Path input = fixture.getTempDir().resolve("final-return-only-input/pkg");
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Files.createDirectories(input);
    Files.copy(jasmClasses.resolve("TestFinalReturnOnlyBase.class"), input.resolve("TestFinalReturnOnlyBase.class"));
    Files.copy(jasmClasses.resolve("TestFinalReturnOnlyInterface.class"), input.resolve("TestFinalReturnOnlyInterface.class"));
    Files.copy(jasmClasses.resolve("TestFinalReturnOnlyChild.class"), input.resolve("TestFinalReturnOnlyChild.class"));

    String content = decompileDirectory(input.getParent(), "pkg/TestFinalReturnOnlyChild.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("String"), content);

    recompile();
  }
}
