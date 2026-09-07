package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class SyntheticAccessorPreservationRegressionTest extends DecompileRegressionTestBase {
  private void copyJasmClass(String simpleName, Path inputRoot) throws IOException {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path input = inputRoot.resolve("pkg");
    Files.createDirectories(input);
    Path classFile = input.resolve(simpleName + ".class");
    Files.copy(jasmClasses.resolve(simpleName + ".class"), classFile);
  }

  private void copyClassFamily(String simpleName, Path sourcePackage, Path inputRoot) throws IOException {
    Path input = inputRoot.resolve("pkg");
    Files.createDirectories(input);
    try (var stream = Files.list(sourcePackage)) {
      for (Path classFile : stream
        .filter(path -> {
          String fileName = path.getFileName().toString();
          return fileName.equals(simpleName + ".class") || (fileName.startsWith(simpleName + "$") && fileName.endsWith(".class"));
        })
        .toList()) {
        Files.copy(classFile, input.resolve(classFile.getFileName()));
      }
    }
  }

  @Test
  public void testSyntheticAccessorReferencedBySeparateClassStillCompiles() throws IOException {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path input = fixture.getTempDir().resolve("synthetic-accessor-input/pkg");
    Files.createDirectories(input);
    Files.copy(jasmClasses.resolve("TestSyntheticAccessorOwner.class"), input.resolve("TestSyntheticAccessorOwner.class"));
    Files.copy(jasmClasses.resolve("TestSyntheticAccessorOwner$1.class"), input.resolve("TestSyntheticAccessorOwner$1.class"));

    String owner = decompileDirectory(input.getParent(), "pkg/TestSyntheticAccessorOwner.java");
    assertFalse(owner.contains("$VF: Couldn't be decompiled"), owner);
    assertTrue(owner.contains("access$000(TestSyntheticAccessorOwner"), owner);

    String nested = Files.readString(fixture.getTargetDir().resolve("pkg/TestSyntheticAccessorOwner$1.java"));
    assertTrue(nested.contains("TestSyntheticAccessorOwner.access$000"), nested);

    recompile();
  }

  @Test
  public void testSameClassSyntheticMethodReferencedByVisibleCodeStillCompiles() throws IOException {
    Path inputRoot = fixture.getTempDir().resolve("synthetic-same-class-input");
    copyJasmClass("TestSyntheticSameClassHiddenMethod", inputRoot);

    String content = decompileDirectory(inputRoot, "pkg/TestSyntheticSameClassHiddenMethod.java");
    assertTrue(content.contains("static String hidden()"), content);

    recompile();
  }

  @Test
  public void testTransitiveSyntheticMethodDependencyStillCompiles() throws IOException {
    Path inputRoot = fixture.getTempDir().resolve("synthetic-transitive-input");
    copyJasmClass("TestSyntheticTransitiveHiddenMethod", inputRoot);

    String content = decompileDirectory(inputRoot, "pkg/TestSyntheticTransitiveHiddenMethod.java");
    assertTrue(content.contains("static String first()"), content);
    assertTrue(content.contains("static String second()"), content);

    recompile();
  }

  @Test
  public void testSyntheticConstructorMarkerArgumentIsElidedInsteadOfPreserved() throws IOException {
    Path inputRoot = fixture.getTempDir().resolve("synthetic-constructor-marker-input");
    copyClassFamily("TestInner2", fixture.getTestDataDir().resolve("classes/java8/pkg"), inputRoot);

    String content = decompileDirectory(inputRoot, "pkg/TestInner2.java");
    assertTrue(content.contains("super();"), content);
    assertTrue(content.contains("super(2);"), content);
    assertFalse(content.contains("TestInner2$1"), content);
    assertFalse(content.contains("super(null)"), content);

    recompile();
  }

  @Test
  public void testSyntheticAccessorOwnedByNonStaticInnerClassStillCompiles() throws Exception {
    Path inputRoot = fixture.getTempDir().resolve("non-static-inner-synthetic-accessor-input");
    copyJasmClass("TestNonStaticInnerSyntheticAccessor", inputRoot);
    copyJasmClass("TestNonStaticInnerSyntheticAccessor$Inner", inputRoot);
    copyJasmClass("TestNonStaticInnerSyntheticAccessor$Inner$1", inputRoot);

    assertInnerAccessorBehavior(inputRoot);
    String content = decompileDirectory(inputRoot, "pkg/TestNonStaticInnerSyntheticAccessor.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);

    recompile();
    assertInnerAccessorBehavior(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void assertInnerAccessorBehavior(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> outer = loader.loadClass("pkg.TestNonStaticInnerSyntheticAccessor");
      Object first = outer.getConstructor().newInstance();
      Object second = outer.getConstructor().newInstance();
      Runnable firstTask = (Runnable)outer.getMethod("make").invoke(first);
      Runnable secondTask = (Runnable)outer.getMethod("make").invoke(second);
      firstTask.run();
      secondTask.run();
      firstTask.run();
      assertEquals(2, outer.getField("calls").get(first));
      assertEquals(1, outer.getField("calls").get(second));
    }
  }

  @Test
  public void testLegacyNamedLocalClassUsesAllocationOwner() throws Exception {
    Path source = writeSource("pkg/LegacyLocal.java", """
      package pkg;
      public class LegacyLocal {
        public int calls;
        public Runnable make(final int amount) {
          class Task implements Runnable {
            public void run() { calls += amount; }
          }
          return new Task();
        }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    Path localClass = outRoot().resolve("pkg/LegacyLocal$1Task.class");
    Files.write(localClass, ClassFileTestUtil.removeClassAttribute(Files.readAllBytes(localClass), "EnclosingMethod"));
    for (Path file : java.util.List.of(localClass, outRoot().resolve("pkg/LegacyLocal.class"))) {
      byte[] bytes = Files.readAllBytes(file);
      ClassFileTestUtil.putU2(bytes, 6, 48); // Pre-Java-5 ownership metadata, with unchanged executable code.
      Files.write(file, bytes);
    }
    assertLocalClassBehavior(outRoot());
    String content = decompileDirectory(outRoot(), "pkg/LegacyLocal.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    recompile();
    assertLocalClassBehavior(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void assertLocalClassBehavior(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> outer = loader.loadClass("pkg.LegacyLocal");
      Object instance = outer.getConstructor().newInstance();
      Runnable first = (Runnable)outer.getMethod("make", int.class).invoke(instance, 2);
      Runnable second = (Runnable)outer.getMethod("make", int.class).invoke(instance, 3);
      first.run();
      second.run();
      first.run();
      assertEquals(7, outer.getField("calls").get(instance));
    }
  }

  @Test
  public void testEcjConstructorMarkerCanBeAnOrdinaryMemberClass() throws Exception {
    Path input = fixture.getTempDir().resolve("ecj-constructor-input");
    copyJasmClass("TestOldECJInner", input);
    copyJasmClass("TestOldECJInner$Inner", input);
    assertEcjConstruction(input);
    String content = decompileDirectory(input, "pkg/TestOldECJInner.java");
    assertTrue(content.contains("get().new Inner()"), content);
    recompile();
    assertEcjConstruction(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void assertEcjConstruction(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> owner = Class.forName("pkg.TestOldECJInner", true, loader);
      var method = owner.getDeclaredMethod("test");
      method.setAccessible(true);
      Object result = method.invoke(null);
      assertEquals("pkg.TestOldECJInner$Inner", result.getClass().getName());
      var outer = result.getClass().getDeclaredField("this$0");
      outer.setAccessible(true);
      var instance = owner.getDeclaredField("INSTANCE");
      instance.setAccessible(true);
      assertTrue(outer.get(result) == instance.get(null));
    }
  }

  @Test
  public void testConstructorMarkerDoesNotChangeOrdinaryOverloadCalls() throws Exception {
    Path input = fixture.getTempDir().resolve("constructor-marker-scope");
    copyJasmClass("TestConstructorMarkerScope", input);
    assertEquals(7, invokeMarkerScope(input, "run"));
    assertEquals(5, invokeMarkerScope(input, "runConstructorOverload"));
    String content = decompileDirectory(input, "pkg/TestConstructorMarkerScope.java");
    assertTrue(content.contains("new TestConstructorMarkerScope()"), content);
    recompile();
    assertEquals(7, invokeMarkerScope(fixture.getTempDir().resolve("recompiled-out"), "run"));
    assertEquals(5, invokeMarkerScope(fixture.getTempDir().resolve("recompiled-out"), "runConstructorOverload"));
  }

  private static int invokeMarkerScope(Path classes, String method) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      return (int)Class.forName("pkg.TestConstructorMarkerScope", true, loader).getMethod(method).invoke(null);
    }
  }

  @Test
  public void testPrivateConstructorOverloadsInTheSameSourceNest() throws Exception {
    Path source = writeSource("pkg/TestPrivateConstructorOverloads.java", """
      package pkg;
      public class TestPrivateConstructorOverloads {
        private static class Box {
          final int value;
          private Box(CharSequence value) { this.value = 5; }
          private Box(String value) { this.value = 9; }
        }
        public static int run() { return new Box((CharSequence)null).value; }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    assertPrivateConstructorOverload(outRoot());
    decompileDirectory(outRoot(), "pkg/TestPrivateConstructorOverloads.java");
    recompile();
    assertPrivateConstructorOverload(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void assertPrivateConstructorOverload(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      assertEquals(5, Class.forName("pkg.TestPrivateConstructorOverloads", true, loader).getMethod("run").invoke(null));
    }
  }
}
