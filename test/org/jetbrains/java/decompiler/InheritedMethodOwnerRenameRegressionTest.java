package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class InheritedMethodOwnerRenameRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[]{IFernflowerPreferences.RENAME_ENTITIES, "1"};
  }

  @Test
  public void testInheritedInvocationUsesRenamedDeclaringMethodName() throws IOException {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path baseClass = jasmClasses.resolve("TestInheritedReturnOnlyConflictBase.class");
    Path midClass = jasmClasses.resolve("TestInheritedReturnOnlyConflictMid.class");
    Path testClass = jasmClasses.resolve("TestInheritedReturnOnlyConflict.class");
    assertTrue(Files.isRegularFile(baseClass), "Missing test class: " + baseClass);
    assertTrue(Files.isRegularFile(midClass), "Missing test class: " + midClass);
    assertTrue(Files.isRegularFile(testClass), "Missing test class: " + testClass);

    Path sourceRoot = fixture.getTempDir().resolve("owner-rename-input/pkg");
    Files.createDirectories(sourceRoot);
    Files.copy(baseClass, sourceRoot.resolve(baseClass.getFileName()));
    Files.copy(midClass, sourceRoot.resolve(midClass.getFileName()));
    Files.copy(testClass, sourceRoot.resolve(testClass.getFileName()));

    String content = decompileDirectory(sourceRoot.getParent(), "pkg/TestInheritedReturnOnlyConflict.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(content.contains("return this.a().toString().length();"), content);

    recompile();
  }

  @Test
  public void testInheritedInvocationUsesRenamedDeclaringMethodNameWithRenamedClasses() throws IOException {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm");
    Path classZz1 = jasmClasses.resolve("zz1.class");
    Path classZz2 = jasmClasses.resolve("zz2.class");
    Path classZz3 = jasmClasses.resolve("zz3.class");
    assertTrue(Files.isRegularFile(classZz1), "Missing test class: " + classZz1);
    assertTrue(Files.isRegularFile(classZz2), "Missing test class: " + classZz2);
    assertTrue(Files.isRegularFile(classZz3), "Missing test class: " + classZz3);

    Path sourceRoot = fixture.getTempDir().resolve("owner-rename-obf-input");
    Files.createDirectories(sourceRoot);
    Files.copy(classZz1, sourceRoot.resolve(classZz1.getFileName()));
    Files.copy(classZz2, sourceRoot.resolve(classZz2.getFileName()));
    Files.copy(classZz3, sourceRoot.resolve(classZz3.getFileName()));

    fixture.getDecompiler().addSource(sourceRoot.toFile());
    fixture.getDecompiler().decompileContext();

    List<Path> javaSources = listJavaSources(fixture.getTargetDir());
    String testContent = null;
    for (Path source : javaSources) {
      String content = DecompilerTestFixture.getContent(source);
      if (content.contains("int test()")) {
        testContent = content;
        break;
      }
    }

    assertNotNull(testContent, "Could not locate decompiled class containing test() method");
    assertFalse(testContent.contains("$VF: Couldn't be decompiled"), testContent);
    assertFalse(testContent.contains("return this.a().toString().length();"), testContent);

    recompile();
  }

  @Test
  public void testFieldLookupDoesNotRewriteOwnerLocalFieldToRenamedSuperclassField() throws IOException {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm");
    Path classYy1 = jasmClasses.resolve("yy1.class");
    Path classYy2 = jasmClasses.resolve("yy2.class");
    assertTrue(Files.isRegularFile(classYy1), "Missing test class: " + classYy1);
    assertTrue(Files.isRegularFile(classYy2), "Missing test class: " + classYy2);

    Path sourceRoot = fixture.getTempDir().resolve("owner-rename-field-input");
    Files.createDirectories(sourceRoot);
    Files.copy(classYy1, sourceRoot.resolve(classYy1.getFileName()));
    Files.copy(classYy2, sourceRoot.resolve(classYy2.getFileName()));

    fixture.getDecompiler().addSource(sourceRoot.toFile());
    fixture.getDecompiler().decompileContext();

    List<Path> javaSources = listJavaSources(fixture.getTargetDir());
    String fieldContent = null;
    for (Path source : javaSources) {
      String content = DecompilerTestFixture.getContent(source);
      if (content.contains("testField()")) {
        fieldContent = content;
        break;
      }
    }

    assertNotNull(fieldContent, "Could not locate decompiled class containing testField()");
    assertFalse(fieldContent.contains("$VF: Couldn't be decompiled"), fieldContent);

    recompile();
  }
}
