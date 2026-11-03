package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.lang.reflect.Array;
import java.nio.file.Path;
import java.util.Arrays;

public class StackSavedAccumulatorRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void preservesTotalAcrossRecordValueAndAdjustmentSlotReuse() throws Exception {
    Path original = fixture.getTestDataDir().resolve("classes/jasm");
    decompileClassFile(original.resolve("pkg/TestStackSavedAccumulator.class"), "pkg/TestStackSavedAccumulator.java");
    recompile();
    try (BehaviorComparison behavior = compareBehavior(original)) {
      for (int[] initial : new int[][]{{}, {100}, {100, 200}, {-5, 7, 11}}) {
        for (int demand : new int[]{0, 20, -20, 3, -3}) {
          int[] expectedValues = initial.clone();
          int expectedTotal = 0;
          int expectedDemand = demand;
          for (int i = 0; i < initial.length; i++) {
            int updated = initial[i] + expectedDemand;
            expectedDemand -= (updated - initial[i]) * 50 / 100;
            expectedTotal += updated;
            expectedValues[i] = updated;
          }
          Object[] expected = {new int[]{expectedTotal, expectedDemand}, expectedValues};
          behavior.assertSame("records=" + Arrays.toString(initial) + ", demand=" + demand, expected, loader -> {
            Class<?> type = loader.loadClass("pkg.TestStackSavedAccumulator");
            Object records = Array.newInstance(type, initial.length);
            for (int i = 0; i < initial.length; i++) {
              Array.set(records, i, type.getConstructor(int.class).newInstance(initial[i]));
            }
            Object result = type.getMethod("accumulate", records.getClass(), int.class).invoke(null, records, demand);
            int[] values = new int[initial.length];
            for (int i = 0; i < initial.length; i++) values[i] = type.getField("value").getInt(Array.get(records, i));
            return new Object[]{result, values};
          });
        }
      }
    }
  }
}
