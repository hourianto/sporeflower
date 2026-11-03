package org.jetbrains.java.decompiler;

import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

import java.nio.file.Path;

public class DuplicatedRandomSeedRegressionTest extends DecompileRegressionTestBase {
  @ParameterizedTest(name = "preserves duplicated next seed: shortRange={0}")
  @ValueSource(booleans = {true, false})
  public void preservesNextSeedAcrossLocalOverwrite(boolean shortRange) throws Exception {
    Path original = fixture.getTestDataDir().resolve("classes/jasm");
    decompileClassFile(original.resolve("pkg/TestDuplicatedRandomSeed.class"), "pkg/TestDuplicatedRandomSeed.java");
    recompile();
    try (BehaviorComparison behavior = compareBehavior(original)) {
      for (int initialSeed : new int[]{1, 0, -1, Integer.MIN_VALUE, Integer.MAX_VALUE}) {
        int expectedSeed = initialSeed;
        int[] expectedValues = new int[4];
        int[] expectedSeeds = new int[4];

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
        }

        // The first return value alone agrees even when the saved seed is
        // corrupt. Check both state and successive results, without reseeding.
        behavior.assertSame("shortRange=" + shortRange + ", initialSeed=" + initialSeed,
          new Object[]{expectedValues, expectedSeeds}, loader -> {
            Class<?> type = loader.loadClass("pkg.TestDuplicatedRandomSeed");
            Object state = type.getConstructor().newInstance();
            type.getField("state").set(null, state);
            var seedField = type.getField("seed");
            seedField.setInt(state, initialSeed);
            var next = type.getMethod("next", boolean.class);
            int[] values = new int[4], seeds = new int[4];
            for (int call = 0; call < values.length; call++) {
              values[call] = (int)next.invoke(null, shortRange);
              seeds[call] = seedField.getInt(state);
            }
            return new Object[]{values, seeds};
          });
      }
    }
  }
}
