package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.lang.reflect.InvocationTargetException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Hashtable;
import java.util.List;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class ConstructorPreinitHelperRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testStraightLinePreinitMovesIntoConstructorArgumentHelpers() throws Exception {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path testClass = jasmClasses.resolve("TestConstructorPreinitMultiArg.class");
    Path baseClass = jasmClasses.resolve("TestMissingSubclassConstructorBase.class");
    assertTrue(Files.isRegularFile(testClass), "Missing test class: " + testClass);
    assertTrue(Files.isRegularFile(baseClass), "Missing support class: " + baseClass);

    Path input = fixture.getTempDir().resolve("classes/pkg");
    Files.createDirectories(input);
    Files.copy(testClass, input.resolve("TestConstructorPreinitMultiArg.class"));
    Files.copy(baseClass, input.resolve("TestMissingSubclassConstructorBase.class"));
    assertParsedArguments(input.getParent());

    String content = decompileDirectory(input.getParent(), "pkg/TestConstructorPreinitMultiArg.java");

    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(content.contains("source-only stub"), content);
    assertTrue(content.contains("super($sporeflower$preinit$0(var1), $sporeflower$preinit$1(var1));"), content);
    assertTrue(content.contains("private static int $sporeflower$preinit$0"), content);
    assertTrue(content.contains("private static int $sporeflower$preinit$1"), content);

    recompile();
    assertParsedArguments(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void assertParsedArguments(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("pkg.TestConstructorPreinitMultiArg");
      var left = type.getSuperclass().getDeclaredField("left");
      var right = type.getSuperclass().getDeclaredField("right");
      left.setAccessible(true);
      right.setAccessible(true);
      List<Map<String, Integer>> inputs = List.of(
        Map.of("id", 10, "from_id", 20),
        Map.of("post_id", 30, "owner_id", 40),
        Map.of("id", -1, "post_id", 31, "from_id", 0, "owner_id", 41),
        Map.of(),
        Map.of("id", 0, "from_id", -2)
      );
      int[][] expected = {{10, 20}, {30, 40}, {31, 41}, {0, 0}, {0, -2}};
      for (int i = 0; i < inputs.size(); i++) {
        Object instance = type.getConstructor(Hashtable.class).newInstance(new Hashtable<>(inputs.get(i)));
        assertEquals(expected[i][0], left.getInt(instance));
        assertEquals(expected[i][1], right.getInt(instance));
      }
    }
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
  public void testLookupThrowPreinitMovesIntoThisArgumentHelper() throws Exception {
    Path input = copyJasmClasses(
      "TestConstructorPreinitLookupThis",
      "TestConstructorPreinitLookupInput",
      "TestConstructorPreinitLookupSpecial"
    );
    assertLookupArguments(input.getParent());

    String content = decompileDirectory(input.getParent(), "pkg/TestConstructorPreinitLookupThis.java");

    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("this(var1, $sporeflower$preinit$0(var1));"), content);
    assertTrue(content.contains("throw new IllegalArgumentException(\"unknown\");"), content);

    recompile();
    assertLookupArguments(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void assertLookupArguments(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("pkg.TestConstructorPreinitLookupThis");
      Class<?> inputType = loader.loadClass("pkg.TestConstructorPreinitLookupInput");
      var constructor = type.getConstructor(inputType);
      var inputField = type.getDeclaredField("input");
      var sizeField = type.getDeclaredField("size");
      inputField.setAccessible(true);
      sizeField.setAccessible(true);
      for (Object input : List.of(inputType.getConstructor(String.class).newInstance("MD5"),
        loader.loadClass("pkg.TestConstructorPreinitLookupSpecial").getConstructor().newInstance())) {
        Object instance = constructor.newInstance(input);
        assertSame(input, inputField.get(instance));
        assertEquals(64, sizeField.getInt(instance));
      }
      Object unknown = inputType.getConstructor(String.class).newInstance("unknown");
      InvocationTargetException failure = assertThrows(InvocationTargetException.class, () -> constructor.newInstance(unknown));
      assertInstanceOf(IllegalArgumentException.class, failure.getCause());
    }
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
