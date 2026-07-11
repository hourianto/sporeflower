package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class IncompleteClassMethodResolutionRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void inheritedDefaultMethodDoesNotProduceFailureStub() throws IOException {
    String content = compileDecompileAndRead(
      "pkg/TestInheritedDefaultMethod.java",
      """
        package pkg;

        public class TestInheritedDefaultMethod {
          interface Parent {
            void act();
          }

          interface Child extends Parent {
            default void act() {
            }
          }

          static final class Implementation implements Child {
          }
        }
        """
    );

    assertFalse(content.contains("AbstractMethodError"), content);
    recompile();
  }

  @Test
  public void inheritedClassMethodDoesNotProduceFailureStub() throws IOException {
    String content = compileDecompileAndRead(
      "pkg/TestInheritedClassMethod.java",
      """
        package pkg;

        public class TestInheritedClassMethod {
          interface Contract {
            void act();
          }

          static class Base {
            public void act() {
            }
          }

          static final class Implementation extends Base implements Contract {
          }
        }
        """
    );

    assertFalse(content.contains("AbstractMethodError"), content);
    recompile();
  }

  @Test
  public void inheritedImplementationIsNotHiddenFromSource() throws IOException {
    Path classes = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path input = fixture.getTempDir().resolve("required-hidden-method/pkg");
    Files.createDirectories(input);
    for (String className : List.of(
      "TestRequiredHiddenInterface",
      "TestRequiredHiddenBase",
      "TestRequiredHiddenChild"
    )) {
      Files.copy(classes.resolve(className + ".class"), input.resolve(className + ".class"));
    }

    String content = decompileDirectory(input.getParent(), "pkg/TestRequiredHiddenBase.java");
    assertFalse(content.contains("AbstractMethodError"), content);
    assertTrue(content.contains("return 7;"), content);
    recompile();
  }

  @Test
  public void legacyInterfaceFailureIsSharedByAbstractAdapter() throws Exception {
    Path classes = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path input = fixture.getTempDir().resolve("legacy-missing-method/pkg");
    Files.createDirectories(input);
    for (String className : List.of(
      "TestLegacyMissingInterface",
      "TestLegacyMissingAdapter",
      "TestLegacyMissingFirstChild",
      "TestLegacyMissingSecondChild"
    )) {
      Files.copy(classes.resolve(className + ".class"), input.resolve(className + ".class"));
    }

    decompileDirectory(input.getParent(), "pkg/TestLegacyMissingAdapter.java");
    String adapter = Files.readString(fixture.getTargetDir().resolve("pkg/TestLegacyMissingAdapter.java"));
    String firstChild = Files.readString(fixture.getTargetDir().resolve("pkg/TestLegacyMissingFirstChild.java"));
    String secondChild = Files.readString(fixture.getTargetDir().resolve("pkg/TestLegacyMissingSecondChild.java"));
    assertTrue(adapter.contains("public void act()"), adapter);
    assertTrue(adapter.contains("throw new AbstractMethodError();"), adapter);
    assertFalse(firstChild.contains("void act()"), firstChild);
    assertFalse(secondChild.contains("void act()"), secondChild);

    recompile();
    for (Path runtimeClasses : List.of(
      fixture.getTestDataDir().resolve("classes/jasm"),
      fixture.getTempDir().resolve("recompiled-out")
    )) {
      assertInvocationThrows(
        runtimeClasses,
        AbstractMethodError.class,
        "pkg.TestLegacyMissingFirstChild",
        new Class<?>[0],
        new Object[0],
        "pkg.TestLegacyMissingInterface",
        "act",
        new Class<?>[0],
        new Object[0]
      );
    }
  }

  @Test
  public void legacyFailureIsNotSharedAcrossModernDefaultDescendant() throws IOException {
    Path classes = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path input = fixture.getTempDir().resolve("mixed-version-missing-method/pkg");
    Files.createDirectories(input);
    for (String className : List.of(
      "TestLegacyMissingInterface",
      "TestLegacyMissingAdapter",
      "TestLegacyMissingFirstChild",
      "TestModernDefaultInterface",
      "TestModernDefaultChild"
    )) {
      Files.copy(classes.resolve(className + ".class"), input.resolve(className + ".class"));
    }

    decompileDirectory(input.getParent(), "pkg/TestLegacyMissingAdapter.java");
    String adapter = Files.readString(fixture.getTargetDir().resolve("pkg/TestLegacyMissingAdapter.java"));
    String brokenChild = Files.readString(fixture.getTargetDir().resolve("pkg/TestLegacyMissingFirstChild.java"));
    String defaultChild = Files.readString(fixture.getTargetDir().resolve("pkg/TestModernDefaultChild.java"));
    assertFalse(adapter.contains("void act()"), adapter);
    assertTrue(brokenChild.contains("public void act()"), brokenChild);
    assertFalse(defaultChild.contains("void act()"), defaultChild);
    recompile();
  }
}
