package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class InlinePreIncrementIntoPostIncrementArrayIndexRegressionTest extends DecompileRegressionTestBase {
  private static final Pattern NESTED_INCREMENT = Pattern.compile("\\(\\+\\+var\\d+\\)\\+\\+");
  private static final Pattern DELAYED_AFTER_CALL = Pattern.compile("side\\(\\)\\s*==\\s*\\+\\+var\\d+");
  private static final Pattern CONDITIONAL_SHORT_CIRCUIT_INCREMENT = Pattern.compile("&&\\s*\\+\\+var\\d+");

  @Override
  protected Object[] fixtureOptions() {
    return new Object[] {IFernflowerPreferences.J2ME_STRICT_SLOT_MERGE, "1"};
  }

  @Test
  public void testPreIncrementIsNotInlinedIntoPostIncrementArrayIndex() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestInlinePreIncrementIntoPostIncrementArrayIndex.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestInlinePreIncrementIntoPostIncrementArrayIndex.java");

    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(NESTED_INCREMENT.matcher(content).find(), content);

    Path decompiledFile = fixture.getTargetDir().resolve("pkg/TestInlinePreIncrementIntoPostIncrementArrayIndex.java");
    if (!Files.isRegularFile(decompiledFile)) {
      decompiledFile = fixture.getTargetDir().resolve("TestInlinePreIncrementIntoPostIncrementArrayIndex.java");
    }
    compileJava8(decompiledFile, fixture.getTempDir().resolve("compile-out"));
  }

  @Test
  public void testPreIncrementIsNotDelayedPastEarlierConditionEvaluation() throws IOException {
    String content = compileDecompileAndRead("pkg/TestInlinePreIncrementEvaluationOrder.java",
      """
        package pkg;

        public class TestInlinePreIncrementEvaluationOrder {
          static int side() { return 0; }
          static void hit() {}

          public static int compareAfterCall(int i) {
            ++i;
            if (side() == i) { hit(); }
            return i;
          }

          public static int shortCircuitAfterCall(int i) {
            ++i;
            if (side() != 0 && i > 0) { hit(); }
            return i;
          }
        }
        """);

    assertFalse(DELAYED_AFTER_CALL.matcher(content).find(), content);
    assertFalse(CONDITIONAL_SHORT_CIRCUIT_INCREMENT.matcher(content).find(), content);
    recompile();
  }
}
