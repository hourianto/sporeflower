package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.net.URLClassLoader;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class FinalFieldMergedReceiverAssignmentRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testFinalFieldAssignmentThroughMergedReceiverCompiles() throws Exception {
    String content = decompileFixture("TestFinalFieldMergedReceiverAssignment");
    assertTrue(content.contains("private final int value;"), content);
    assertTrue(content.contains("this.value ="), content);

    recompile();

    Path originalClasses = fixture.getTestDataDir().resolve("classes/jasm");
    Path recompiledClasses = recompiledRoot();
    assertEquals(readMergedReceiverValue(originalClasses, true), readMergedReceiverValue(recompiledClasses, true));
    assertEquals(readMergedReceiverValue(originalClasses, false), readMergedReceiverValue(recompiledClasses, false));
  }

  @Test
  public void testReceiverThatMayNotBeThisRendersFieldMutable() throws Exception {
    String content = decompileFixture("TestFinalFieldAmbiguousReceiver");
    assertFalse(content.contains("final int value"), content);
    assertTrue(content.contains("int value"), content);
    assertTrue(content.contains(".value = 7;"), content);
    assertFalse(content.contains("this.value = 7;"), content);

    recompile();

    Path originalClasses = fixture.getTestDataDir().resolve("classes/jasm");
    Path recompiledClasses = recompiledRoot();
    assertEquals(readAmbiguousReceiverValues(originalClasses, false), readAmbiguousReceiverValues(recompiledClasses, false));
    assertEquals(readAmbiguousReceiverValues(originalClasses, true), readAmbiguousReceiverValues(recompiledClasses, true));
  }

  @Test
  public void testLoopCarriedReceiverCopiesRemainReceiverEquivalent() throws Exception {
    String content = decompileFixture("TestFinalFieldLoopReceiver");
    assertTrue(content.contains("private final int value;"), content);
    assertTrue(content.contains("this.value ="), content);

    recompile();

    Path original = fixture.getTestDataDir().resolve("classes/jasm");
    Path recompiled = recompiledRoot();
    assertEquals(readIntConstructorValue(original, "pkg.TestFinalFieldLoopReceiver", 0),
      readIntConstructorValue(recompiled, "pkg.TestFinalFieldLoopReceiver", 0));
    assertEquals(readIntConstructorValue(original, "pkg.TestFinalFieldLoopReceiver", 3),
      readIntConstructorValue(recompiled, "pkg.TestFinalFieldLoopReceiver", 3));
  }

  @Test
  public void testReceiverProvenanceDoesNotEraseStaticTypeSemantics() throws Exception {
    Path source = writeSource("pkg/TestReceiverAliasStaticType.java", """
      package pkg;

      class TestReceiverAliasBase {
        int token = 11;

        int choose(TestReceiverAliasBase value) {
          return 1;
        }

        int choose(TestReceiverAliasStaticType value) {
          return 2;
        }
      }

      public class TestReceiverAliasStaticType extends TestReceiverAliasBase {
        int token = 22;
        private final int overload;
        private final int field;

        public TestReceiverAliasStaticType() {
          TestReceiverAliasBase alias = this;
          this.overload = this.choose(alias);
          this.field = alias.token;
        }

        public int value() {
          return this.overload * 100 + this.field;
        }

        public static int run() {
          return new TestReceiverAliasStaticType().value();
        }
      }
      """);
    compileJava8WithDebug(source, outRoot());
    String content = decompileDirectory(outRoot(), "pkg/TestReceiverAliasStaticType.java");
    assertTrue(content.contains("private final int overload"), content);
    assertTrue(content.contains("private final int field"), content);
    assertTrue(content.contains("TestReceiverAliasBase alias = this"), content);

    recompile();

    Path original = outRoot();
    Path recompiled = recompiledRoot();
    assertEquals(111, readStaticInt(original, "pkg.TestReceiverAliasStaticType", "run"));
    assertEquals(111, readStaticInt(recompiled, "pkg.TestReceiverAliasStaticType", "run"));
  }

  @Test
  public void testForeignSameNamedFieldKeepsItsOwner() throws Exception {
    Path input = copyJasmClasses("TestForeignStaticFinalOwner", "TestForeignStaticFinalTarget");
    String content = decompileDirectory(input, "pkg/TestForeignStaticFinalOwner.java");
    readDecompiled("pkg/TestForeignStaticFinalTarget.java");
    assertTrue(content.contains("TestForeignStaticFinalTarget.value = 1"), content);

    recompile();
  }


  private static int readMergedReceiverValue(Path classes, boolean branch) throws Exception {
    try (URLClassLoader loader = loader(classes)) {
      Class<?> type = Class.forName("pkg.TestFinalFieldMergedReceiverAssignment", true, loader);
      Object instance = type.getConstructor(boolean.class).newInstance(branch);
      return (Integer)type.getMethod("value").invoke(instance);
    }
  }

  private static String readAmbiguousReceiverValues(Path classes, boolean branch) throws Exception {
    try (URLClassLoader loader = loader(classes)) {
      Class<?> type = Class.forName("pkg.TestFinalFieldAmbiguousReceiver", true, loader);
      Object target = type.getConstructor(int.class).newInstance(3);
      Object instance = type.getConstructor(boolean.class, type).newInstance(branch, target);
      int instanceValue = (Integer)type.getMethod("value").invoke(instance);
      int targetValue = (Integer)type.getMethod("value").invoke(target);
      return instanceValue + ":" + targetValue;
    }
  }

  private static int readStaticInt(Path classes, String className, String methodName) throws Exception {
    try (URLClassLoader loader = loader(classes)) {
      Class<?> type = Class.forName(className, true, loader);
      return (Integer)type.getMethod(methodName).invoke(null);
    }
  }

  private static int readIntConstructorValue(Path classes, String className, int argument) throws Exception {
    try (URLClassLoader loader = loader(classes)) {
      Class<?> type = Class.forName(className, true, loader);
      Object instance = type.getConstructor(int.class).newInstance(argument);
      return (Integer)type.getMethod("value").invoke(instance);
    }
  }
}
