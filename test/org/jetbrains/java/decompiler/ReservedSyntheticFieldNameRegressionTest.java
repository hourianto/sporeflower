package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

import java.io.IOException;

import static org.junit.jupiter.api.Assertions.assertEquals;

public class ReservedSyntheticFieldNameRegressionTest extends DecompileRegressionTestBase {
  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  public void legacySourceControlsSyntheticThisFieldName(boolean legacy) throws IOException {
    configureOptions(IFernflowerPreferences.LEGACY_SOURCE_COMPATIBILITY, legacy ? "1" : "0");
    String content = decompileFixture("TestSyntheticReferencedField");
    assertEquals(!legacy, content.contains("this$0"), content);
  }
}
