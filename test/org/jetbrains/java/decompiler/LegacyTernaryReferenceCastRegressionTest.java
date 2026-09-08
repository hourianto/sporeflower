package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.lang.reflect.Array;
import java.lang.reflect.InvocationTargetException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.*;

public class LegacyTernaryReferenceCastRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testSiblingReferenceTernaryCastsOneBranchForLegacySource() throws Exception {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path input = fixture.getTempDir().resolve("legacy-ternary-input/pkg");
    Files.createDirectories(input);
    for (String name : new String[]{"TestLegacyTernaryReferenceCast", "TestLegacyTernaryReferenceCastBase",
      "TestLegacyTernaryReferenceCastLeft", "TestLegacyTernaryReferenceCastRight", "TestLegacyArrayTernary"}) {
      Files.copy(jasmClasses.resolve(name + ".class"), input.resolve(name + ".class"));
    }

    String content = decompileDirectory(input.getParent(), "pkg/TestLegacyTernaryReferenceCast.java");

    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(
      content.contains("? (TestLegacyTernaryReferenceCastBase)(new TestLegacyTernaryReferenceCastLeft()) : new TestLegacyTernaryReferenceCastRight()"),
      content
    );
    assertTrue(
      content.contains("? (Object)(new TestLegacyTernaryReferenceCastLeft()) : new byte[1]"),
      content
    );
    assertTrue(
      content.contains("? (Object)(new byte[1]) : new TestLegacyTernaryReferenceCastLeft()"),
      content
    );
    assertTrue(
      content.contains("? (Object[])(new String[1]) : new Integer[1]"),
      content
    );
    assertTrue(content.contains("(TestLegacyTernaryReferenceCastBase)var2 != var3"), content);
    assertTrue(content.contains("return var0 == var1;"), content);
    assertTrue(content.contains("return (Object[])var0 != var1;"), content);
    assertTrue(content.contains("return (Object)var0 != var1;"), content);

    String arrays = Files.readString(fixture.getTargetDir().resolve("pkg/TestLegacyArrayTernary.java"));
    assertAll(
      () -> assertTrue(arrays.contains("? (TestLegacyTernaryReferenceCastBase[])left : right"), arrays),
      () -> assertTrue(arrays.contains("(Object[])"), arrays)
    );

    recompile();
    assertArrayBehavior(input.getParent());
    assertArrayBehavior(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void assertArrayBehavior(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("pkg.TestLegacyArrayTernary");
      Object left = Array.newInstance(loader.loadClass("pkg.TestLegacyTernaryReferenceCastLeft"), 2);
      Object right = Array.newInstance(loader.loadClass("pkg.TestLegacyTernaryReferenceCastRight"), 3);
      type.getField("left").set(null, left);
      type.getField("right").set(null, right);
      assertSame(left, type.getMethod("choose", int.class).invoke(null, 0));
      assertSame(right, type.getMethod("choose", int.class).invoke(null, 1));

      var length = type.getMethod("length", Object[].class, short[].class);
      for (short mode : new short[]{1, 3, 4}) {
        boolean matrix = mode == 3 || mode == 4;
        Object selected = matrix ? new String[3][] : new String[2];
        assertEquals(matrix ? 3 : 2, length.invoke(null, new Object[]{selected}, new short[]{0, 0, mode}));
        assertSame(matrix ? null : selected, type.getField("lines").get(null));
        assertSame(matrix ? selected : null, type.getField("rows").get(null));
        Object beforeLines = type.getField("lines").get(null);
        Object beforeRows = type.getField("rows").get(null);
        for (Object invalid : new Object[]{null, new Object()}) {
          InvocationTargetException failure = assertThrows(InvocationTargetException.class,
            () -> length.invoke(null, new Object[]{invalid}, new short[]{0, 0, mode}));
          assertEquals(invalid == null ? NullPointerException.class : ClassCastException.class, failure.getCause().getClass());
          assertSame(beforeLines, type.getField("lines").get(null));
          assertSame(beforeRows, type.getField("rows").get(null));
        }
      }
    }
  }
}
