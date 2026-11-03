package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;

import static org.junit.jupiter.api.Assertions.assertTrue;

public class StatementLevelUnboxingPopRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testStatementLevelUnboxingIsRenderedAsInvocation() throws IOException {
    String content = decompileFixture("TestStatementLevelUnboxingPop");
    assertTrue(content.contains(".intValue();"), content);

    recompile();
  }
}
