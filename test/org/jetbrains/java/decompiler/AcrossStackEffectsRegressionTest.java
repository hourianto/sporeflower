package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertThrows;

public class AcrossStackEffectsRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void preservesOrderAcrossStatements() throws Exception {
    Path original = fixture.getTestDataDir().resolve("classes/jasm");
    checkBehavior(original);
    decompileClassFile(original.resolve("pkg/TestAcrossStackEffects.class"), "pkg/TestAcrossStackEffects.java");
    recompile();
    checkBehavior(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void checkBehavior(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("pkg.TestAcrossStackEffects");
      Object receiver = type.getConstructor().newInstance();

      type.getField("calls").setInt(null, 0);
      assertEquals(7 + 1 * 10 + 2 * 100, type.getMethod("pair", int.class, type).invoke(receiver, 7, receiver), classes + ": pair order");
      assertEquals(2, type.getField("calls").getInt(null), classes + ": pair calls");

      checkDivision(type, "divideBeforeEffect", 6, 3, 2 * 100 + 9, classes);

      assertEquals(15 * 100 + 16, type.getMethod("localWrite", int.class).invoke(null, 5), classes + ": localWrite");
    }
  }

  // The division runs before effect(), so a zero divisor must throw without calling it.
  private static void checkDivision(Class<?> type, String name, int x, int y, int expected, Path classes) throws Exception {
    Method method = type.getMethod(name, int.class, int.class);

    type.getField("calls").setInt(null, 0);
    assertEquals(expected, method.invoke(null, x, y), classes + ": " + name);
    assertEquals(1, type.getField("calls").getInt(null), classes + ": " + name + " calls");

    type.getField("calls").setInt(null, 0);
    InvocationTargetException failure = assertThrows(InvocationTargetException.class, () -> method.invoke(null, x, 0));
    assertInstanceOf(ArithmeticException.class, failure.getCause(), classes + ": " + name + " by zero");
    assertEquals(0, type.getField("calls").getInt(null), classes + ": " + name + " calls before throwing");
  }
}
