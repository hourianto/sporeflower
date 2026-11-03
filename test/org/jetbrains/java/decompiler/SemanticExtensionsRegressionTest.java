package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;

public class SemanticExtensionsRegressionTest extends SemanticTestBase {
  public SemanticExtensionsRegressionTest() {
    super("extensions", "ExtensionSubject");
  }

  @Test
  public void exclusiveFlagSelectorsNeverBecomeCombinationsOfOtherModes() throws IOException {
    String content = subject("""
      static int validPacked() { return 65539; }
      static int unknownPacked() { return 65542; }
      static int clearedFlag() { return -65537; }
      """);
    assertContains(content, "Packed.PHONE");
    assertEquals(3, countOccurrences(content, "Packed.ENABLED"), content);
    assertContains(content, "~Packed.ENABLED");
    assertFalse(content.contains("Packed.NUMBER") || content.contains("Packed.URL"), content);
    recompile();
  }

  @Test
  public void recordsPropagateAcrossReadsWritesAliasesAndNestedArrays() throws IOException {
    String content = subject("""
      int[] records = {2, 1, 2, 2};
      int[][] rows = {{2, 1}};
      boolean state(int i) { return records[i * 2] == 2; }
      boolean flags(int i) { return records[(i << 1) + 1] == 2; }
      boolean alias(int i) { int[] data = records; return data[i * 2] == 2; }
      boolean row(int r, int i) { return rows[r][i * 2] == 2; }
      void write(int i) { records[i * 2] = 2; records[i * 2 + 1] = 1; }
      boolean dynamic(int i) { return records[i] == 2; }
      """);
    assertContains(content, "{State.READY, Mask.READ, State.READY, Mask.WRITE}");
    assertEquals(3, countOccurrences(content, "== State.READY"), content);
    assertEquals(1, countOccurrences(content, "== Mask.WRITE"), content);
    assertContains(content, "Fields.FLAGS", "= State.READY;", "= Mask.READ;");
    assertEquals(1, countOccurrences(content, "== 2"), content);
    recompile();
  }

  @Test
  public void headersAndNonPowerOfTwoStridesRetainOverflowUncertainty() throws IOException {
    String content = subject("""
      int[] counted = {2, 2, 1, 2, 2};
      int[] triples = {2, 1, 0, 2, 2, 0};
      boolean safe(int i) { return counted[i * 2 + 1] == 2; }
      boolean headerPossible(int i) { return counted[i * 2 + 2] == 2; }
      boolean header() { return counted[0] == 2; }
      boolean fixed() { return triples[3] == 2; }
      boolean overflow(int i) { return triples[i * 3] == 2; }
      """);
    assertContains(content, "{2, State.READY, Mask.READ, State.READY, Mask.WRITE}", "counted[Header.COUNT] == 2");
    assertEquals(2, countOccurrences(content, "== State.READY"), content);
    assertEquals(3, countOccurrences(content, "== 2"), content);
    recompile();
  }

  @Test
  public void callDomainsStayOnTheSelectedInvocationIncludingNestedCalls() throws IOException {
    // javac emits iload_0 (offset 0), invokestatic read (1). A nested
    // read(read(i)) adds the outer invokestatic at offset 4.
    String content = subject("""
      static int read(int value) { return value; }
      static boolean scoped(int input) {
        int typed = read(input);
        int raw = read(input);
        return typed == 2 && raw == 2;
      }
      static boolean nested(int input) { return read(read(input)) == 2; }
      static boolean secondNested(int input) { return read(read(input)) == 2; }
      static boolean wrongCallee(int input) { return read(input) == 2; }
      static boolean unbound(int input) { return read(input) == 2; }
      """);
    assertEquals(2, countOccurrences(content, "== State.READY"), content);
    assertEquals(4, countOccurrences(content, "== 2"), content);
    assertNotContains(content, "return State.READY");
    recompile();
  }

  @Test
  public void callParameterDomainsStayOnTheirArgumentAndInvocation() throws IOException {
    String content = subject("""
      ExtensionSubject(int value) {}
      static int send(int kind, int value) { return value; }
      static int read(int value) { return value; }
      static int callArguments() { return send(2, 2) + send(2, 2); }
      static int nestedArguments() { return send(read(2), 2); }
      static ExtensionSubject constructorArgument() { return new ExtensionSubject(2); }
      """);
    assertContains(content, "send(Mask.WRITE, State.READY) + send(2, 2)", "send(read(Mask.WRITE), State.READY)",
      "new ExtensionSubject(State.READY)");
    recompile();
  }
}
