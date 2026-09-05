package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class SemanticExtensionsRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[]{IFernflowerPreferences.SEMANTIC_MAPPINGS_PATH, "testData/semantic/extensions.json"};
  }

  @Test
  public void exclusiveFlagSelectorsNeverBecomeCombinationsOfOtherModes() throws IOException {
    String content = subject("""
      static int validPacked() { return 65539; }
      static int unknownPacked() { return 65542; }
      static int clearedFlag() { return -65537; }
      """);
    assertTrue(content.contains("Packed.PHONE"), content);
    assertEquals(3, count(content, "Packed.ENABLED"), content);
    assertTrue(content.contains("~Packed.ENABLED"), content);
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
    assertTrue(content.contains("{State.READY, Mask.READ, State.READY, Mask.WRITE}"), content);
    assertEquals(3, count(content, "== State.READY"), content);
    assertEquals(1, count(content, "== Mask.WRITE"), content);
    assertTrue(content.contains("Fields.FLAGS"), content);
    assertTrue(content.contains("= State.READY;"), content);
    assertTrue(content.contains("= Mask.READ;"), content);
    assertEquals(1, count(content, "== 2"), content);
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
    assertTrue(content.contains("{2, State.READY, Mask.READ, State.READY, Mask.WRITE}"), content);
    assertTrue(content.contains("counted[Header.COUNT] == 2"), content);
    assertEquals(2, count(content, "== State.READY"), content);
    assertEquals(3, count(content, "== 2"), content);
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
    assertEquals(2, count(content, "== State.READY"), content);
    assertEquals(4, count(content, "== 2"), content);
    assertFalse(content.contains("return State.READY"), content);
    recompile();
  }

  private String subject(String body) throws IOException {
    return compileDecompileAndRead("sample/ExtensionSubject.java", "package sample; public class ExtensionSubject {\n" + body + "\n}");
  }

  private static int count(String text, String needle) {
    return (text.length() - text.replace(needle, "").length()) / needle.length();
  }
}
