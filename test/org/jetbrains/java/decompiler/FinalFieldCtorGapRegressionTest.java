package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class FinalFieldCtorGapRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testFinalFieldWithoutAssignmentInAllConstructorsCompiles() throws IOException {
    decompileFixture("TestFinalFieldCtorGap");
    recompile();
  }

  @Test
  public void testStaticFinalFieldWithoutInitializerCompiles() throws IOException {
    String content = decompileFixture("TestStaticFinalFieldNoInitializer");
    assertFalse(content.contains("static final int G"), content);
    assertTrue(content.contains("static int G"), content);

    recompile();
  }

  @Test
  public void testStaticFinalFieldWithoutDefiniteInitializerCompiles() throws IOException {
    String content = decompileFixture("TestStaticFinalFieldPartialInitializer");
    assertFalse(content.contains("static final int G"), content);
    assertTrue(content.contains("static int G"), content);

    recompile();
  }
}
