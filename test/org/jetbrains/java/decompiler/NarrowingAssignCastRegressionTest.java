package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;


public class NarrowingAssignCastRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testNarrowingReferenceAssignmentsKeepRequiredCast() throws IOException {
    String content = decompileFixture("TestNarrowingAssignCast");

    Path supportSource = fixture.getTempDir().resolve("compile-src/pkg/SupportTypes.java");
    Files.createDirectories(supportSource.getParent());
    Files.writeString(
      supportSource,
      """
        package pkg;

        class TestNarrowingBase {
          static TestNarrowingBase produce() {
            return null;
          }
        }

        class TestNarrowingDerived extends TestNarrowingBase {
          void ping() {
          }
        }
        """
    );

    recompile(List.of(supportSource));
  }
}
