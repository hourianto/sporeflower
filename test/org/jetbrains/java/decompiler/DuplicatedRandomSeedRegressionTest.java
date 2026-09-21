package org.jetbrains.java.decompiler;

import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertAll;
import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;

public class DuplicatedRandomSeedRegressionTest extends DecompileRegressionTestBase {
  @ParameterizedTest(name = "preserves duplicated next seed: shortRange={0}")
  @ValueSource(booleans = {true, false})
  public void preservesNextSeedAcrossLocalOverwrite(boolean shortRange) throws Exception {
    Path original = fixture.getTestDataDir().resolve("classes/jasm");
    // Execute the assembled input first to check JVM validity and the oracle,
    // independently of whether the decompiler produces compilable source.
    checkSequences(original, shortRange, "original bytecode");

    String source = decompileClassFile(original.resolve("pkg/TestDuplicatedRandomSeed.class"),
      "pkg/TestDuplicatedRandomSeed.java");
    assertFalse(source.contains("Couldn't be decompiled"), source);
    recompile();
    checkSequences(fixture.getTempDir().resolve("recompiled-out"), shortRange, "recompiled source:\n" + source);
  }

  private static void checkSequences(Path classes, boolean shortRange, String context) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("pkg.TestDuplicatedRandomSeed");
      var seedField = type.getField("seed");
      var next = type.getMethod("next", boolean.class);

      for (int initialSeed : new int[]{1, 0, -1, Integer.MIN_VALUE, Integer.MAX_VALUE}) {
        Object state = type.getConstructor().newInstance();
        type.getField("state").set(null, state);
        seedField.setInt(state, initialSeed);
        int expectedSeed = initialSeed;
        int[] expectedValues = new int[4];
        int[] expectedSeeds = new int[4];
        int[] actualValues = new int[4];
        int[] actualSeeds = new int[4];

        for (int call = 0; call < expectedValues.length; call++) {
          if (shortRange) {
            // The category-1 path wraps at 32 bits before the signed shift.
            expectedSeed = expectedSeed * 214013 + 2531011;
            expectedValues[call] = (expectedSeed >> 16) & 32767;
          } else {
            // The category-2 path sign-extends the old seed, extracts from the
            // full long result, and only then narrows the saved state to int.
            long intermediate = (long)expectedSeed * 214013L + 2531011L;
            expectedValues[call] = (int)(intermediate >> 32) & Integer.MAX_VALUE;
            expectedSeed = (int)intermediate;
          }
          expectedSeeds[call] = expectedSeed;
          actualValues[call] = (int)next.invoke(null, shortRange);
          actualSeeds[call] = seedField.getInt(state);
        }

        // The first return value alone agrees even when the saved seed is
        // corrupt. Check both state and successive results, without reseeding.
        assertAll(context + "\nshortRange=" + shortRange + ", initialSeed=" + initialSeed,
          () -> assertArrayEquals(expectedValues, actualValues, "successive return values"),
          () -> assertArrayEquals(expectedSeeds, actualSeeds, "saved seed after each call"));
      }
    }
  }
}
