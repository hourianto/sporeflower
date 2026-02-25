package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class GetClassPopNullCheckRetentionRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testGetClassPopNullCheckIsNotDropped() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestGetClassPopNullCheckRetention.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestGetClassPopNullCheckRetention.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(
      content.contains("maybe.getClass();") || content.contains("Objects.requireNonNull(maybe);"),
      content
    );

    Path decompiledFile = fixture.getTargetDir().resolve("pkg/TestGetClassPopNullCheckRetention.java");
    if (!Files.isRegularFile(decompiledFile)) {
      decompiledFile = fixture.getTargetDir().resolve("TestGetClassPopNullCheckRetention.java");
    }
    compileJava8(decompiledFile, fixture.getTempDir().resolve("compile-out"));
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
