package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class ConstructorPreinitHelperRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testStraightLinePreinitMovesIntoConstructorArgumentHelpers() throws IOException {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path testClass = jasmClasses.resolve("TestConstructorPreinitMultiArg.class");
    Path baseClass = jasmClasses.resolve("TestMissingSubclassConstructorBase.class");
    assertTrue(Files.isRegularFile(testClass), "Missing test class: " + testClass);
    assertTrue(Files.isRegularFile(baseClass), "Missing support class: " + baseClass);

    Path input = fixture.getTempDir().resolve("classes/pkg");
    Files.createDirectories(input);
    Files.copy(testClass, input.resolve("TestConstructorPreinitMultiArg.class"));
    Files.copy(baseClass, input.resolve("TestMissingSubclassConstructorBase.class"));

    String content = decompileDirectory(input.getParent(), "pkg/TestConstructorPreinitMultiArg.java");

    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(content.contains("source-only stub"), content);
    assertTrue(content.contains("super($sporeflower$preinit$0(var1), $sporeflower$preinit$1(var1));"), content);
    assertTrue(content.contains("private static int $sporeflower$preinit$0"), content);
    assertTrue(content.contains("private static int $sporeflower$preinit$1"), content);

    recompile();
  }

  @Test
  public void testBranchingThisFactoryMovesIntoConstructorArgumentHelper() throws IOException {
    Path input = copyJasmClasses(
      "TestConstructorPreinitThisFactory",
      "TestConstructorPreinitThisFactoryItem",
      "TestMissingSubclassConstructorBase"
    );

    String content = decompileDirectory(input.getParent(), "pkg/TestConstructorPreinitThisFactory.java");

    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("this(var2, $sporeflower$preinit$0(var1, var2, var3));"), content);
    assertTrue(content.contains("private static Object $sporeflower$preinit$0"), content);
    assertTrue(content.contains("new TestConstructorPreinitThisFactoryItem"), content);
    assertTrue(content.contains("new StringBuffer"), content);

    recompile();
  }

  @Test
  public void testMutatingPreinitMovesIntoSuperArgumentHelper() throws IOException {
    Path input = copyJasmClasses(
      "TestConstructorPreinitMutatingSuperArg",
      "TestConstructorPreinitHashtableBase"
    );

    String content = decompileDirectory(input.getParent(), "pkg/TestConstructorPreinitMutatingSuperArg.java");

    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("super($sporeflower$preinit$0(var1));"), content);
    assertTrue(content.contains(".put(\"pid\""), content);
    assertTrue(content.contains(".remove(\"gid\")"), content);

    recompile();
  }

  @Test
  public void testThrowingPreinitMovesIntoConstructorArgumentHelper() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestConstructorPreinitThrow.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestConstructorPreinitThrow.java");

    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("this($sporeflower$preinit$0(var1));"), content);
    assertTrue(content.contains("throw new IllegalArgumentException(\"bad\");"), content);

    recompile();
  }

  @Test
  public void testCheckedThrowingPreinitParticipatesInExceptionAnalysis() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestConstructorPreinitChecked.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestConstructorPreinitChecked.java");

    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("this($sporeflower$preinit$0(var1));"), content);
    assertTrue(content.contains("private static int $sporeflower$preinit$0"), content);
    assertTrue(content.contains("throws IOException"), content);

    recompile();
  }

  @Test
  public void testLookupThrowPreinitMovesIntoThisArgumentHelper() throws IOException {
    Path input = copyJasmClasses(
      "TestConstructorPreinitLookupThis",
      "TestConstructorPreinitLookupInput",
      "TestConstructorPreinitLookupSpecial"
    );

    String content = decompileDirectory(input.getParent(), "pkg/TestConstructorPreinitLookupThis.java");

    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("this(var1, $sporeflower$preinit$0(var1));"), content);
    assertTrue(content.contains("throw new IllegalArgumentException(\"unknown\");"), content);

    recompile();
  }

  @Test
  public void testInterleavedSideEffectsAreNotLiftedIntoReorderedHelpers() throws IOException {
    Path input = copyJasmClasses(
      "TestConstructorPreinitInterleavedSideEffects",
      "TestMissingSubclassConstructorBase"
    );

    String content = decompileDirectory(input.getParent(), "pkg/TestConstructorPreinitInterleavedSideEffects.java");

    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("tick(0)"), content);
    assertTrue(content.contains("tick(1)"), content);
    assertTrue(content.contains("tick(2)"), content);
    assertTrue(content.contains("tick(3)"), content);
    assertTrue(content.indexOf("tick(0)") < content.indexOf("tick(1)"), content);
    assertTrue(content.indexOf("tick(1)") < content.indexOf("tick(2)"), content);
    assertTrue(content.indexOf("tick(2)") < content.indexOf("tick(3)"), content);
  }

  private Path copyJasmClasses(String... classNames) throws IOException {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path input = fixture.getTempDir().resolve("classes/pkg");
    Files.createDirectories(input);
    for (String className : classNames) {
      Path classFile = jasmClasses.resolve(className + ".class");
      assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);
      Files.copy(classFile, input.resolve(className + ".class"));
    }
    return input;
  }
}
