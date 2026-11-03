package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;

public class SemanticPropagationRegressionTest extends SemanticTestBase {
  public SemanticPropagationRegressionTest() {
    super("propagation", "PropagationSubject");
  }

  @Test
  public void arithmeticDeltasAndRangeBoundsStayNumeric() throws Exception {
    String content = subject("""
      int direction;
      public static int rotate(int direction) {
        direction -= 2;
        if (direction < 0) direction += 8;
        return direction;
      }
      public static int rotateExpanded(int direction) {
        direction = direction - 2;
        if (0 > direction) direction = direction + 8;
        return direction;
      }
      void update() { direction -= 2; direction <<= 2; }
      void wrap() { direction -= 2; if (direction < 0) direction += 8; }
      void reverseWrap() { if (0 > direction) direction = direction + 8; }
      void assign() { direction = 2; }
      boolean equal() { return direction == 0; }
      boolean threshold(int direction) { return direction >= 2; }
      boolean notSentinel() { return direction >= 0; }
      boolean bounds() { return direction < -1 || direction > 6; }
      """);
    assertContains(content, "-= 2", "<<= 2");
    assertFalse(content.contains("-= Direction.") || content.contains("<<= Direction."), content);
    assertContains(content, "= Direction.SOUTH;", "== Direction.WEST", ">= Direction.SOUTH", ">= 0");
    assertNotContains(content, ">= Direction.WEST");
    assertFalse(content.contains("< Direction.WEST") || content.contains("Direction.WEST >"), content);
    assertContains(content, "> Direction.NORTH");
    recompile();
    assertSameIntResults("rotate", -8, 0, 1, 2, 7, 10, Integer.MIN_VALUE, Integer.MAX_VALUE);
    assertSameIntResults("rotateExpanded", -8, 0, 1, 2, 7, 10, Integer.MIN_VALUE, Integer.MAX_VALUE);
  }

  @Test
  public void unknownAlternativesDoNotAcquireKnownDomains() throws IOException {
    String content = subject("""
      int direction;
      int[] states;
      boolean scalar(boolean choose, int raw) { return (choose ? direction : raw) == 2; }
      boolean local(boolean choose, int raw) {
        int value;
        if (choose) value = direction; else value = raw;
        return value == 2;
      }
      boolean array(boolean choose, int[] raw) { return (choose ? states : raw)[0] == 2; }
      boolean known(boolean choose) { return (choose ? direction : states[0]) == 2; }
      """);
    assertEquals(1, countOccurrences(content, "== Direction.SOUTH"), content);
    assertEquals(3, countOccurrences(content, "== 2"), content);
    recompile();
  }

  @Test
  public void bitwiseAmbiguitySurvivesNestedExpressions() throws IOException {
    String content = subject("""
      int mask;
      int otherMask;
      boolean mixed() { return ((mask | otherMask) & mask) == 2; }
      boolean preserveMask(int mask) { mask &= 3; return mask == 1; }
      boolean compound() { return (mask |= 2) == 1; }
      """);
    assertContains(content, "== 2", "|= Mask.WRITE");
    assertEquals(2, countOccurrences(content, "== Mask.READ"), content);
    recompile();
  }

  @Test
  public void conflictingProducerAndConsumerContextsStayNumeric() throws IOException {
    String content = subject("""
      int mask;
      int otherMask;
      void copy() { mask = otherMask | 2; }
      """);
    assertContains(content, "| 2");
    assertFalse(content.contains("Mask.WRITE") || content.contains("OtherMask.ACTIVE"), content);
    recompile();
  }

  @Test
  public void comparisonsDoNotTruncateLiteralsToTheFieldsStorageWidth() throws Exception {
    String content = subject("""
      byte narrowMask;
      public static boolean outsideByteRange(int input) {
        PropagationSubject value = new PropagationSubject();
        value.narrowMask = (byte)input;
        return value.narrowMask == 129;
      }
      public static boolean withinByteRange(int input) {
        PropagationSubject value = new PropagationSubject();
        value.narrowMask = (byte)input;
        return value.narrowMask == -127;
      }
      """);
    recompile();
    assertSameIntResults("outsideByteRange", -127, 0, 1, 128, 129, 255);
    assertSameIntResults("withinByteRange", -127, 0, 1, 128, 129, 255);
    assertContains(content, "Mask.READ");
  }

  @Test
  public void castsMustPreserveIntegralValuesBeforePropagating() throws Exception {
    String content = subject("""
      int direction;
      int wideValue;
      int mask;
      public static boolean floating(int input) {
        PropagationSubject value = new PropagationSubject();
        value.direction = input;
        return (double)value.direction == 2.5;
      }
      boolean narrowing() { return (byte)wideValue == 2; }
      boolean narrowKnownValues() { return (byte)direction == 2; }
      boolean widening() { return (long)direction == 2L; }
      public static int maskedCast(int input) {
        PropagationSubject value = new PropagationSubject();
        value.mask = (byte)(input | 257);
        return value.mask;
      }
      """);
    assertContains(content, "== 2.5", "== 2;");
    assertEquals(2, countOccurrences(content, "== Direction.SOUTH"), content);
    recompile();
    assertSameIntResults("floating", 0, 2, 3, Integer.MAX_VALUE);
    assertSameIntResults("maskedCast", 0, 2, 128, 256, -1);
  }

  @Test
  public void arrayConsumersReachReturnsBranchesAndOuterSlots() throws IOException {
    String content = subject("""
      int[][] rows = new int[][]{{2}, {2}};
      int[] create() { return new int[]{2}; }
      int[] chooseArray(boolean choose) { return choose ? new int[]{0} : new int[]{2}; }
      void consume(int[] values) {}
      void pass(boolean choose) { consume(choose ? new int[]{0} : new int[]{2}); }
      void replace() { rows[0] = new int[]{2}; }
      """);
    assertContains(content, "new int[][]{{Direction.SOUTH}, {2}}");
    assertEquals(5, countOccurrences(content, "{Direction.SOUTH}"), content);
    assertEquals(2, countOccurrences(content, "{Direction.WEST}"), content);
    recompile();
  }

  @Test
  public void dynamicSlotsRetainConflictsWithoutLosingSharedIndexMeanings() throws IOException {
    String content = subject("""
      int[][] mixedRows;
      boolean dynamic(int row) { return mixedRows[row][0] == 2; }
      boolean specific() { return mixedRows[0][0] == 2; }
      boolean fallback() { return mixedRows[1][0] == 2; }
      """);
    assertContains(content, "[var1][Direction.WEST] == 2", "[Slots.STATE][Direction.WEST] == Direction.SOUTH",
      "[1][Direction.WEST] == Mask.WRITE");
    recompile();
  }

  @Test
  public void freshNumericArraysCanJoinKnownContractsButDynamicInitializersCannot() throws IOException {
    String content = subject("""
      int[] states;
      boolean fresh(boolean choose) {
        int[] values = choose ? states : new int[]{2};
        return values[0] == 2;
      }
      boolean allocated(boolean choose) {
        int[] values = choose ? states : new int[1];
        return values[0] == 2;
      }
      boolean dynamic(boolean choose, int raw) {
        int[] values = choose ? states : new int[]{raw};
        return values[0] == 2;
      }
      """);
    assertEquals(2, countOccurrences(content, "== Direction.SOUTH"), content);
    assertEquals(1, countOccurrences(content, "== 2"), content);
    assertContains(content, "new int[]{Direction.SOUTH}");
    recompile();
  }

  @Test
  public void nearestExplicitOverrideReplacesAncestorContracts() throws IOException {
    Path root = writeSource("sample/BindingRoot.java", """
      package sample;
      class BindingRoot {
        boolean choose(int value) { return value == 2; }
        int convert(int value) { return value; }
      }
      """);
    Path middle = writeSource("sample/BindingMiddle.java", """
      package sample;
      class BindingMiddle extends BindingRoot {
        boolean choose(int value) { return value == 2; }
        int convert(int value) { return value; }
      }
      """);
    Path leaf = writeSource("sample/BindingLeaf.java", """
      package sample;
      public class BindingLeaf extends BindingMiddle {
        boolean choose(int value) { return value == 2; }
        int convert(int value) { return value; }
        boolean call(int raw) { return convert(raw) == 2; }
      }
      """);
    compileJava8NoDebug(List.of(root, middle, leaf), outRoot());
    String content = decompileDirectory(outRoot(), "sample/BindingLeaf.java");
    assertContains(content, "== Mask.WRITE");
    assertNotContains(content, "Direction.SOUTH");
    recompile();
  }
}
