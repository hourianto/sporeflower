package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

import java.nio.file.Path;

public class ParameterWideningTest extends DecompileRegressionTestBase {
  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  public void entryValuesRetainTheirTypesWhileWritablePhisCanWiden(boolean strict) throws Exception {
    configureOptions(IFernflowerPreferences.J2ME_STRICT_SLOT_MERGE, strict ? "1" : "0");
    Path original = fixture.getTestDataDir().resolve("classes/jasm");
    decompileClassFile(original.resolve("pkg/TestParameterWidening.class"), "pkg/TestParameterWidening.java");
    recompile();

    try (BehaviorComparison behavior = compareBehavior(original)) {
      for (boolean flag : new boolean[]{false, true}) {
        for (int value = Byte.MIN_VALUE; value <= Byte.MAX_VALUE; value++) {
          int expected = (value & 255) + (flag ? 256 : 0);
          if (expected == 511) expected = -1;
          byte input = (byte)value;
          behavior.assertSame("flag=" + flag + ", value=" + value, expected, loader ->
            loader.loadClass("pkg.TestParameterWidening").getMethod("decode", boolean.class, byte.class).invoke(null, flag, input));
        }
      }

      for (int iterations : new int[]{0, 1, 3}) {
        for (char character : new char[]{0, 255, Character.MAX_VALUE}) {
          for (short value : new short[]{Short.MIN_VALUE, 0, Short.MAX_VALUE}) {
            assertResult(behavior, "loop", new Class<?>[]{long.class, char.class, short.class, int.class},
              17 + character + value + iterations * 255, 17L, character, value, iterations);
          }
        }
        for (byte value : new byte[]{Byte.MIN_VALUE, 0, Byte.MAX_VALUE}) {
          assertResult(behavior, "narrow", new Class<?>[]{byte.class, int.class},
            (int)(byte)(value + iterations), value, iterations);
        }
      }
      for (boolean fail : new boolean[]{false, true}) {
        for (byte value : new byte[]{Byte.MIN_VALUE, 0, Byte.MAX_VALUE}) {
          assertResult(behavior, "caught", new Class<?>[]{byte.class, boolean.class},
            value + (fail ? 400 : 0), value, fail);
        }
        for (boolean value : new boolean[]{false, true}) {
          assertResult(behavior, "booleanSlot", new Class<?>[]{boolean.class, boolean.class},
            fail ? 3 : value ? 1 : 0, value, fail);
        }
        assertResult(behavior, "reference", new Class<?>[]{String.class, boolean.class},
          fail ? 42 : "entry", "entry", fail);
      }
    }
  }

  private static void assertResult(BehaviorComparison behavior, String name, Class<?>[] types,
                                   Object expected, Object... arguments) throws Exception {
    behavior.assertSame(name + java.util.Arrays.deepToString(arguments), expected, loader ->
      loader.loadClass("pkg.TestParameterWidening").getMethod(name, types).invoke(null, arguments));
  }
}
