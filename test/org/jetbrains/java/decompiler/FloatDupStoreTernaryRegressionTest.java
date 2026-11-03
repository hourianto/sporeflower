package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class FloatDupStoreTernaryRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testFloatTernaryWithDupStoreStackMergeDoesNotFallBack() throws Exception {
    String content = decompileFixture("TestFloatDupStoreTernary");
    assertFalse(content.contains("No common supertype for ternary expression"), content);
    assertTrue(content.contains("public static float min4(float"), content);
    assertTrue(content.contains("public static float max4(float"), content);

    recompile();
    // Local names are free to coalesce. Verify the duplicated values survive
    // the nested assignments, including unordered comparisons and signed zero.
    try (BehaviorComparison behavior = compareBehavior(fixture.getTestDataDir().resolve("classes/jasm"))) {
      float[] values = {-Float.MAX_VALUE, -1, -0.0F, 0.0F, 1, Float.MAX_VALUE, Float.NaN, Float.POSITIVE_INFINITY, Float.NEGATIVE_INFINITY};
      Class<?>[] parameters = {float.class, float.class, float.class, float.class};
      for (String name : new String[]{"min4", "max4"}) {
        for (float a : values) for (float b : values) for (float c : values) for (float d : values) {
          // Float equality preserves the same signed-zero/NaN semantics as floatToIntBits.
          behavior.assertSameStaticCall("pkg.TestFloatDupStoreTernary", name, parameters, a, b, c, d);
        }
      }
    }
  }
}
