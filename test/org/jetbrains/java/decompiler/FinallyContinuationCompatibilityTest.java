package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.File;
import java.lang.reflect.InvocationTargetException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;

public class FinallyContinuationCompatibilityTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[]{
      IFernflowerPreferences.IGNORE_INVALID_BYTECODE, "1",
      IFernflowerPreferences.VERIFY_ANONYMOUS_CLASSES, "1",
      IFernflowerPreferences.VERIFY_PRE_POST_VARIABLE_MERGES, "1"
    };
  }

  @Test
  public void resourceReturnsKeepTheirValuesAndExceptions() throws Exception {
    for (String name : List.of("TestTryWithResourcesReturn", "TestTryWithResourcesLoop", "TestTryWithResourcesNestedLoop")) {
      // Use the same Java 8 bytecode as the source snapshots. Compiling with
      // a newer javac's -source 8 produces different resource cleanup graphs.
      Path compiled = outRoot().resolve("pkg/" + name + ".class");
      Files.createDirectories(compiled.getParent());
      Files.copy(fixture.getTestDataDir().resolve("classes/java8/pkg/" + name + ".class"), compiled);
      if (name.equals("TestTryWithResourcesReturn")) {
        // The existing snapshot records a structuring failure for this
        // unrelated method. Keep the tested methods' bytecode unchanged.
        Files.write(compiled, ClassFileTestUtil.removeMethods(Files.readAllBytes(compiled), "testFinallyNested"));
      }
    }
    String content = decompileDirectory(outRoot(), "pkg/TestTryWithResourcesReturn.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    recompile();

    Path empty = fixture.getTempDir().resolve("empty");
    Path tokens = fixture.getTempDir().resolve("tokens");
    Path loop = fixture.getTempDir().resolve("loop");
    Path missing = fixture.getTempDir().resolve("missing");
    Files.writeString(empty, "");
    Files.writeString(tokens, "one two three");
    Files.writeString(loop, "0 tail");
    try (URLClassLoader original = loader(outRoot());
         URLClassLoader recompiled = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      for (Path first : List.of(empty, tokens, missing)) {
        for (Path second : List.of(empty, tokens, missing)) {
          for (String method : List.of("testComplex", "testComplex1", "testComplex2")) {
            Object[] args = {first.toFile(), second.toFile(), loop.toFile()};
            Class<?>[] types = {File.class, File.class, File.class};
            assertEquals(call(original, "TestTryWithResourcesReturn", method, types, args),
              call(recompiled, "TestTryWithResourcesReturn", method, types, args), method + ": " + first + ", " + second);
          }
          Class<?>[] types = {Path.class, Path.class, int.class, int.class, int.class, int.class, int.class};
          Object[] args = {first, second, 1, 0, 0, 0, 0};
          assertEquals(call(original, "TestTryWithResourcesNestedLoop", "test", types, args),
            call(recompiled, "TestTryWithResourcesNestedLoop", "test", types, args));
        }
        // With two scanners on the same file this method always breaks or
        // returns on its first iteration, including the missing-file case.
        assertEquals(call(original, "TestTryWithResourcesLoop", "test3", new Class<?>[]{File.class}, new Object[]{first.toFile()}),
          call(recompiled, "TestTryWithResourcesLoop", "test3", new Class<?>[]{File.class}, new Object[]{first.toFile()}));
      }
    }
  }

  @Test
  public void finallyBreakOverridesReturnWithoutAContinuationMerge() throws Exception {
    String content = compileDecompileAndRead("pkg/TestFinallyBreak.java", """
      package pkg;
      public class TestFinallyBreak {
        public int test(int x) {
          do {
            try {
              if (x < 25) return 5;
            } finally {
              if (x > 3) break;
            }
          } while (x < 45);
          return 1;
        }
      }
      """);
    assertFalse(content.contains("semaphore"), content);
    recompile();
    try (URLClassLoader original = loader(outRoot());
         URLClassLoader recompiled = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      for (int x = -1; x <= 50; x++) {
        assertEquals(x <= 3 ? 5 : 1,
          call(original, "TestFinallyBreak", "test", new Class<?>[]{int.class}, new Object[]{x}));
        assertEquals(x <= 3 ? 5 : 1,
          call(recompiled, "TestFinallyBreak", "test", new Class<?>[]{int.class}, new Object[]{x}));
      }
    }
  }

  private static URLClassLoader loader(Path root) throws Exception {
    return new URLClassLoader(new URL[]{root.toUri().toURL()}, ClassLoader.getPlatformClassLoader());
  }

  private static Object call(ClassLoader loader, String name, String method, Class<?>[] types, Object[] args) throws Exception {
    Class<?> type = loader.loadClass("pkg." + name);
    try {
      return type.getMethod(method, types).invoke(type.getConstructor().newInstance(), args);
    } catch (InvocationTargetException exception) {
      Throwable cause = exception.getCause();
      return cause.getClass().getName() + ":" + cause.getMessage();
    }
  }
}
