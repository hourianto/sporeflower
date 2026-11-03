package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.nio.file.Path;
import java.util.List;

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
    Path input = copyJasmClasses("TestIncompleteAbstractBase", "TestIncompleteAbstractChild", "TestIncompleteCovariantInterface");
    String content = decompileDirectory(input, "pkg/TestIncompleteAbstractChild.java");
    String baseContent = readDecompiled("pkg/TestIncompleteAbstractBase.java");
    // The String-returning interface method is a separate JVM entry point from
    // Base.image():Object; source covariance must not join unrelated entry points.
    assertTrue(java.util.regex.Pattern.compile("public String (?!image\\()\\w+\\(\\)").matcher(content).find(), content);
    assertTrue(content.contains("throw new AbstractMethodError();"), content);
    assertTrue(baseContent.contains("public void draw()"), baseContent);
    assertTrue(baseContent.contains("throw new AbstractMethodError();"), baseContent);

    recompile();

    List<Path> runtimeClasses = List.of(
      fixture.getTestDataDir().resolve("classes/jasm"),
      recompiledRoot()
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
