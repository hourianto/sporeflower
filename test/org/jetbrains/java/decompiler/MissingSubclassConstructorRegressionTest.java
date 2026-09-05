package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Modifier;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.*;

public class MissingSubclassConstructorRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testSubclassWithoutConstructorAndNoDefaultSuperConstructorCompiles() throws Exception {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path input = fixture.getTempDir().resolve("missing-subclass-constructor-input/pkg");
    Files.createDirectories(input);
    Files.copy(jasmClasses.resolve("TestMissingSubclassConstructorBase.class"), input.resolve("TestMissingSubclassConstructorBase.class"));
    Files.copy(jasmClasses.resolve("TestMissingSubclassConstructorChild.class"), input.resolve("TestMissingSubclassConstructorChild.class"));

    try (URLClassLoader loader = loader(input.getParent())) {
      Class<?> child = loader.loadClass("pkg.TestMissingSubclassConstructorChild");
      assertEquals(0, child.getDeclaredConstructors().length);
      assertNotNull(child.getMethod("value"));
      assertThrows(NoSuchMethodException.class, () -> child.getDeclaredConstructor());
    }
    String content = decompileDirectory(input.getParent(), "pkg/TestMissingSubclassConstructorChild.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);

    recompile();
    assertNotConstructible("pkg.TestMissingSubclassConstructorChild");
    assertTrue(content.contains("final Object value"), content);
  }

  @Test
  public void testConstructorlessUtilityDoesNotGainPublicConstructor() throws Exception {
    Path source = writeSource("pkg/Utility.java", """
      package pkg;
      public final class Utility {
        public static int answer() { return 42; }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    removeConstructors(outRoot().resolve("pkg/Utility.class"));
    try (URLClassLoader loader = loader(outRoot())) {
      Class<?> original = loader.loadClass("pkg.Utility");
      assertEquals(0, original.getDeclaredConstructors().length);
      assertEquals(42, original.getMethod("answer").invoke(null));
    }
    decompileDirectory(outRoot(), "pkg/Utility.java");
    recompile();
    assertNotConstructible("pkg.Utility");
    try (URLClassLoader loader = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      assertEquals(42, loader.loadClass("pkg.Utility").getMethod("answer").invoke(null));
    }
  }

  @Test
  public void testAccessibleSuperOverloadWithWideArgumentsAndCheckedException() throws Exception {
    Path source = writeSource("pkg/Child.java", """
      package pkg;
      class Base {
        private Base() { }
        protected Base(long number, double fraction, Object[] values) throws java.io.IOException { }
      }
      public final class Child extends Base {
        private final Object value;
        public Child() throws java.io.IOException { super(0L, 0D, null); value = null; }
        public Object value() { return value; }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    removeConstructors(outRoot().resolve("pkg/Child.class"));
    try (URLClassLoader loader = loader(outRoot())) {
      assertEquals(0, loader.loadClass("pkg.Child").getDeclaredConstructors().length);
    }
    String content = decompileDirectory(outRoot(), "pkg/Child.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    recompile();
    assertNotConstructible("pkg.Child");
  }

  @Test
  public void testExistingPrivateConstructorKeepsItsBehavior() throws Exception {
    Path source = writeSource("pkg/Existing.java", """
      package pkg;
      public class Existing {
        public final int value;
        private Existing(int value) { this.value = value; }
        public static Existing make() { return new Existing(7); }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    decompileDirectory(outRoot(), "pkg/Existing.java");
    recompile();
    try (URLClassLoader loader = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      Class<?> type = loader.loadClass("pkg.Existing");
      assertEquals(1, type.getDeclaredConstructors().length);
      assertEquals(7, type.getField("value").get(type.getMethod("make").invoke(null)));
    }
  }

  private static void removeConstructors(Path file) throws IOException {
    Files.write(file, ClassFileTestUtil.removeMethods(Files.readAllBytes(file), "<init>"));
  }

  private void assertNotConstructible(String className) throws Exception {
    try (URLClassLoader loader = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      Class<?> child = loader.loadClass(className);
      assertEquals(0, child.getConstructors().length, "Source repair must not make this class publicly instantiable");
      assertEquals(1, child.getDeclaredConstructors().length);
      for (var constructor : child.getDeclaredConstructors()) {
        assertTrue(Modifier.isPrivate(constructor.getModifiers()));
        constructor.setAccessible(true);
        Object[] args = new Object[constructor.getParameterCount()];
        for (int i = 0; i < args.length; i++) {
          if (constructor.getParameterTypes()[i] == int.class) args[i] = 0;
          if (constructor.getParameterTypes()[i] == long.class) args[i] = 0L;
          if (constructor.getParameterTypes()[i] == double.class) args[i] = 0D;
        }
        InvocationTargetException failure = assertThrows(InvocationTargetException.class, () -> constructor.newInstance(args));
        assertInstanceOf(IllegalStateException.class, failure.getCause());
      }
    }
  }

  private static URLClassLoader loader(Path classes) throws IOException {
    return new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader());
  }
}
