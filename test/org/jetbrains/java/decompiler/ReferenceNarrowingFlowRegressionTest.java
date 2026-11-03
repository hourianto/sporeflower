package org.jetbrains.java.decompiler;

import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

import java.io.IOException;
import java.nio.file.Path;
import java.util.List;

public class ReferenceNarrowingFlowRegressionTest extends DecompileRegressionTestBase {
  @ParameterizedTest(name = "{0}")
  @ValueSource(strings = {
    "TestInlineCastAssignmentQualifierProbe",
    "TestInlineTypeRefinementCastProbe",
    "TestSlotReuseNarrowingCastProbe",
    "TestSlotReuseNarrowingCastLoopProbe"
  })
  public void narrowingCastFlowCompiles(String className) throws IOException {
    Path supportSource = writeSource("pkg/SupportTypes.java",
      """
        package pkg;

        class SupportBase {
          int value;

          static SupportBase produce() {
            return null;
          }
        }

        class SupportDerived extends SupportBase {
          int special;

          void ping() {
          }
        }
        """
    );

    decompileFixture(className);
    recompile(List.of(supportSource));
  }
}
