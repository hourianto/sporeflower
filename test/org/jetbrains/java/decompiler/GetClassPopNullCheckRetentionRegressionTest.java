package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class GetClassPopNullCheckRetentionRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testGetClassPopNullCheckIsNotDropped() throws IOException {
    String content = decompileFixture("TestGetClassPopNullCheckRetention");
    assertTrue(
      content.contains("maybe.getClass();") || content.contains("Objects.requireNonNull(maybe);"),
      content
    );

    recompile();
  }

  @Test
  public void testQualifiedNewStillRemovesSyntheticNullCheck() throws IOException {
    String content = compileDecompileAndRead(
      "pkg/TestQualifiedNewNullCheckStillSimplified.java",
      """
        package pkg;

        public class TestQualifiedNewNullCheckStillSimplified {
          static class Outer {
            class Inner {
              Inner() {
              }
            }
          }

          public static void test(Outer outer) {
            outer.new Inner();
          }
        }
        """
    );

    assertFalse(content.contains("getClass();"), content);
    assertFalse(content.contains("requireNonNull("), content);
    assertTrue(content.contains("new Inner();"), content);
  }
}
