package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class IncompleteAbstractSubclassRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[] {
      IFernflowerPreferences.RENAME_ENTITIES, "1",
      IFernflowerPreferences.LEGACY_SOURCE_COMPATIBILITY, "1"
    };
  }

  @Test
  public void testFinalSubclassMissingAbstractSuperclassMethodsCompiles() throws Exception {
    Path baseClass = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestIncompleteAbstractBase.class");
    Path childClass = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestIncompleteAbstractChild.class");
    Path covariantInterface = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestIncompleteCovariantInterface.class");
    assertTrue(Files.isRegularFile(baseClass), "Missing test class: " + baseClass);
    assertTrue(Files.isRegularFile(childClass), "Missing test class: " + childClass);
    assertTrue(Files.isRegularFile(covariantInterface), "Missing test class: " + covariantInterface);

    Path input = fixture.getTempDir().resolve("incomplete-abstract-input/pkg");
    Files.createDirectories(input);
    Files.copy(baseClass, input.resolve(baseClass.getFileName()));
    Files.copy(childClass, input.resolve(childClass.getFileName()));
    Files.copy(covariantInterface, input.resolve(covariantInterface.getFileName()));
    decompileDirectory(input.getParent(), "pkg/TestIncompleteAbstractChild.java");

    Path decompiledFile = fixture.getTargetDir().resolve("pkg/TestIncompleteAbstractChild.java");
    if (!Files.isRegularFile(decompiledFile)) {
      decompiledFile = fixture.getTargetDir().resolve("TestIncompleteAbstractChild.java");
    }
    String content = DecompilerTestFixture.getContent(decompiledFile);
    Path decompiledBase = fixture.getTargetDir().resolve("pkg/TestIncompleteAbstractBase.java");
    if (!Files.isRegularFile(decompiledBase)) {
      decompiledBase = fixture.getTargetDir().resolve("TestIncompleteAbstractBase.java");
    }
    String baseContent = DecompilerTestFixture.getContent(decompiledBase);
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("public String image()"), content);
    assertTrue(content.contains("throw new AbstractMethodError();"), content);
    assertTrue(baseContent.contains("public void draw()"), baseContent);
    assertTrue(baseContent.contains("throw new AbstractMethodError();"), baseContent);

    recompile();

    List<Path> runtimeClasses = List.of(
      fixture.getTestDataDir().resolve("classes/jasm"),
      fixture.getTempDir().resolve("recompiled-out")
    );
    for (Path classes : runtimeClasses) {
      assertInvocationThrows(
        classes,
        AbstractMethodError.class,
        "pkg.TestIncompleteAbstractChild",
        new Class<?>[0],
        new Object[0],
        "pkg.TestIncompleteAbstractBase",
        "draw",
        new Class<?>[0],
        new Object[0]
      );
      assertInvocationThrows(
        classes,
        AbstractMethodError.class,
        "pkg.TestIncompleteAbstractChild",
        new Class<?>[0],
        new Object[0],
        "pkg.TestIncompleteAbstractBase",
        "image",
        new Class<?>[0],
        new Object[0]
      );
    }
  }
}
