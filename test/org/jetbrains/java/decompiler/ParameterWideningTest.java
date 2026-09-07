package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

import java.lang.reflect.Method;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;

public class ParameterWideningTest extends DecompileRegressionTestBase {
  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  public void entryValuesRetainTheirTypesWhileWritablePhisCanWiden(boolean strict) throws Exception {
    DecompilerContext.setProperty(IFernflowerPreferences.J2ME_STRICT_SLOT_MERGE, strict ? "1" : "0");
    Path original = fixture.getTestDataDir().resolve("classes/jasm");
    String content = decompileClassFile(original.resolve("pkg/TestParameterWidening.class"), "pkg/TestParameterWidening.java");
    assertFalse(content.contains("Couldn't be decompiled"), content);
    recompile();

    try (URLClassLoader before = loader(original);
         URLClassLoader after = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      Class<?> originalClass = before.loadClass("pkg.TestParameterWidening");
      Class<?> decompiledClass = after.loadClass("pkg.TestParameterWidening");
      Method originalMethod = originalClass.getMethod("decode", boolean.class, byte.class);
      Method decompiledMethod = decompiledClass.getMethod("decode", boolean.class, byte.class);
      for (boolean flag : new boolean[]{false, true}) {
        for (int value = Byte.MIN_VALUE; value <= Byte.MAX_VALUE; value++) {
          int expected = (value & 255) + (flag ? 256 : 0);
          if (expected == 511) expected = -1;
          assertEquals(expected, originalMethod.invoke(null, flag, (byte)value));
          assertEquals(expected, decompiledMethod.invoke(null, flag, (byte)value), "flag=" + flag + ", value=" + value);
        }
      }

      for (int iterations : new int[]{0, 1, 3}) {
        for (char character : new char[]{0, 255, Character.MAX_VALUE}) {
          for (short value : new short[]{Short.MIN_VALUE, 0, Short.MAX_VALUE}) {
            assertResult(originalClass, decompiledClass, "loop", new Class<?>[]{long.class, char.class, short.class, int.class},
              17 + character + value + iterations * 255, 17L, character, value, iterations);
          }
        }
        for (byte value : new byte[]{Byte.MIN_VALUE, 0, Byte.MAX_VALUE}) {
          assertResult(originalClass, decompiledClass, "narrow", new Class<?>[]{byte.class, int.class},
            (int)(byte)(value + iterations), value, iterations);
        }
      }
      for (boolean fail : new boolean[]{false, true}) {
        for (byte value : new byte[]{Byte.MIN_VALUE, 0, Byte.MAX_VALUE}) {
          assertResult(originalClass, decompiledClass, "caught", new Class<?>[]{byte.class, boolean.class},
            value + (fail ? 400 : 0), value, fail);
        }
        for (boolean value : new boolean[]{false, true}) {
          assertResult(originalClass, decompiledClass, "booleanSlot", new Class<?>[]{boolean.class, boolean.class},
            fail ? 3 : value ? 1 : 0, value, fail);
        }
        assertResult(originalClass, decompiledClass, "reference", new Class<?>[]{String.class, boolean.class},
          fail ? 42 : "entry", "entry", fail);
      }
    }
  }

  private static void assertResult(Class<?> original, Class<?> decompiled, String name, Class<?>[] types,
                                   Object expected, Object... arguments) throws Exception {
    assertEquals(expected, original.getMethod(name, types).invoke(null, arguments), name + " original");
    assertEquals(expected, decompiled.getMethod(name, types).invoke(null, arguments), name + " decompiled");
  }

  private static URLClassLoader loader(Path classes) throws Exception {
    return new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader());
  }
}
