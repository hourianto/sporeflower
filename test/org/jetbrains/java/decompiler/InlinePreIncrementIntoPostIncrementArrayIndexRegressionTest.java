package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class InlinePreIncrementIntoPostIncrementArrayIndexRegressionTest extends DecompileRegressionTestBase {
  private static final Pattern NESTED_INCREMENT = Pattern.compile("\\(\\+\\+var\\d+\\)\\+\\+");
  private static final Pattern DELAYED_AFTER_CALL = Pattern.compile("side\\(\\)\\s*==\\s*\\+\\+var\\d+");
  private static final Pattern CONDITIONAL_SHORT_CIRCUIT_INCREMENT = Pattern.compile("&&\\s*\\+\\+var\\d+");
  private static final Pattern ORDINARY_IF_FOLDED_INCREMENT = Pattern.compile("if \\(\\+\\+var\\d+ > 0 && var\\d+ < 10\\)");
  private static final Pattern LOOP_HEADER_INCREMENT = Pattern.compile("while \\(\\+\\+var\\d+ > 0\\)");
  private static final Pattern INFINITE_LOOP = Pattern.compile("while \\(true\\)");
  private static final Pattern DELAYED_AFTER_OTHER_INCREMENT = Pattern.compile("call\\(var\\d+\\+\\+, \\+\\+var\\d+\\)");
  private static final Pattern RETURN_FIRST_AND_OPERAND_INCREMENT = Pattern.compile("return \\+\\+var\\d+ > 0 && side\\(\\);");
  private static final Pattern RETURN_SECOND_AND_OPERAND_INCREMENT = Pattern.compile("return side\\(\\) && \\+\\+var\\d+ > 0;");

  @Override
  protected Object[] fixtureOptions() {
    return new Object[] {IFernflowerPreferences.J2ME_STRICT_SLOT_MERGE, "1"};
  }

  @Test
  public void testPreIncrementIsNotInlinedIntoPostIncrementArrayIndex() throws IOException {
    String content = decompileFixture("TestInlinePreIncrementIntoPostIncrementArrayIndex");
    assertFalse(NESTED_INCREMENT.matcher(content).find(), content);

    recompile();
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

          public static int loopHeaderAfterCall(int i) {
            while (true) {
              ++i;
              if (side() == i) { return i; }
              hit();
            }
          }
        }
        """);

    assertFalse(DELAYED_AFTER_CALL.matcher(content).find(), content);
    assertFalse(CONDITIONAL_SHORT_CIRCUIT_INCREMENT.matcher(content).find(), content);
    recompile();
  }

  @Test
  public void testPreIncrementStaysSeparateOutsideLoopHeader() throws IOException {
    String content = compileDecompileAndRead("pkg/TestInlinePreIncrementFirstConditionUse.java",
      """
        package pkg;

        public class TestInlinePreIncrementFirstConditionUse {
          static void hit() {}
          static int call(int a, int b) { return a + b; }

          public static int firstAndOperand(int i) {
            ++i;
            if (i > 0 && i < 10) { hit(); }
            return i;
          }

          public static int otherIncrementBeforeTarget(int i, int j) {
            ++i;
            return call(j++, i);
          }
        }
        """);

    assertFalse(ORDINARY_IF_FOLDED_INCREMENT.matcher(content).find(), content);
    assertFalse(DELAYED_AFTER_OTHER_INCREMENT.matcher(content).find(), content);
    recompile();
  }

  @Test
  public void testPreIncrementCanInlineIntoLoopHeaderGuard() throws IOException {
    String content = compileDecompileAndRead("pkg/TestInlinePreIncrementLoopHeaderUse.java",
      """
        package pkg;

        public class TestInlinePreIncrementLoopHeaderUse {
          static void hit() {}

          public static int loopHeader(int i) {
            while (true) {
              ++i;
              if (i <= 0) { return i; }
              hit();
            }
          }
        }
        """);

    assertTrue(LOOP_HEADER_INCREMENT.matcher(content).find(), content);
    assertFalse(INFINITE_LOOP.matcher(content).find(), content);
    recompile();
  }

  @Test
  public void testStackPreIncrementOnlyInlinesIntoAlwaysEvaluatedConditionOperand() throws IOException {
    String content = compileDecompileAndRead("pkg/TestInlinePreIncrementBooleanOperand.java",
      """
        package pkg;

        public class TestInlinePreIncrementBooleanOperand {
          static boolean side() { return true; }

          public static boolean firstAndOperand(int i) {
            ++i;
            return i > 0 && side();
          }

          public static boolean secondAndOperand(int i) {
            ++i;
            return side() && i > 0;
          }
        }
        """);

    assertTrue(RETURN_FIRST_AND_OPERAND_INCREMENT.matcher(content).find(), content);
    assertFalse(RETURN_SECOND_AND_OPERAND_INCREMENT.matcher(content).find(), content);
    recompile();
  }
}
