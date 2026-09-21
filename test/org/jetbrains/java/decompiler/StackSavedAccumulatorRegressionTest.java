package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.lang.reflect.Array;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;

public class StackSavedAccumulatorRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void preservesTotalAcrossRecordValueAndAdjustmentSlotReuse() throws Exception {
    Path original = fixture.getTestDataDir().resolve("classes/jasm");
    checkBehavior(original);
    decompileClassFile(original.resolve("pkg/TestStackSavedAccumulator.class"), "pkg/TestStackSavedAccumulator.java");
    recompile();
    checkBehavior(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void checkBehavior(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("pkg.TestStackSavedAccumulator");
      for (int[] initial : new int[][]{{}, {100}, {100, 200}, {-5, 7, 11}}) {
        for (int demand : new int[]{0, 20, -20, 3, -3}) {
          Object records = Array.newInstance(type, initial.length);
          int[] expectedValues = initial.clone();
          int expectedTotal = 0;
          int expectedDemand = demand;
          for (int i = 0; i < initial.length; i++) {
            Array.set(records, i, type.getConstructor(int.class).newInstance(initial[i]));
            int updated = initial[i] + expectedDemand;
            expectedDemand -= (updated - initial[i]) * 50 / 100;
            expectedTotal += updated;
            expectedValues[i] = updated;
          }
          String context = classes + ": count=" + initial.length + ", demand=" + demand;
          int[] result = (int[])type.getMethod("accumulate", records.getClass(), int.class).invoke(null, records, demand);
          assertArrayEquals(new int[]{expectedTotal, expectedDemand}, result, context);
          for (int i = 0; i < initial.length; i++) {
            assertEquals(expectedValues[i], type.getField("value").getInt(Array.get(records, i)), context + ", record=" + i);
          }
        }
      }
    }
  }
}
