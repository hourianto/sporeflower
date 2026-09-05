package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.*;

public class SemanticAdvancedRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[]{IFernflowerPreferences.SEMANTIC_MAPPINGS_PATH, "testData/semantic/advanced.json"};
  }

  @Test
  public void packedFieldsMatchMasksShiftsAndSignedExtraction() throws IOException {
    String source = subject("""
      int packed, signedPack;
      boolean low() { return (packed & 7) == 1; }
      boolean high() { return ((packed >>> 3) & 31) == 2; }
      boolean maskFirst() { return ((packed & 248) >>> 3) == 2; }
      boolean unsignedUnmasked() { return (packed >>> 3) == 2; }
      boolean wrongMask() { return (packed & 15) == 2; }
      boolean signed() { return ((signedPack << 16) >> 24) == -1; }
      boolean signedCast() { return (byte)(signedPack >>> 8) == -1; }
      static int pack(boolean second, int family) { return family | ((second ? 2 : 1) << 3); }
      """);
    assertTrue(source.contains("== Family.A"), source);
    assertEquals(2, count(source, "== State.READY"), source);
    assertEquals(2, count(source, "== Signed.NEGATIVE"), source);
    assertEquals(2, count(source, "== 2"), source);
    assertTrue(source.contains("? State.READY : State.FIRST"), source);
    recompile();
  }

  @Test
  public void packedSelectorsRequireTheirGuard() throws IOException {
    String source = subject("""
      boolean tagged(int tag) {
        if ((tag & 7) == 1) return (tag >>> 3) == 2;
        if ((tag & 7) == 2) return (tag >>> 3) == 2;
        return (tag >>> 3) == 2;
      }
      boolean taggedSwitch(int tag) {
        switch (tag & 7) {
          case 1: return (tag >>> 3) == 2;
          case 2: return (tag >>> 3) == 2;
          default: return (tag >>> 3) == 2;
        }
      }
      """);
    assertEquals(2, count(source, "== State.READY"), source);
    assertEquals(2, count(source, "== Other.RUN"), source);
    assertEquals(2, count(source, "== 2"), source);
    recompile();
  }

  @Test
  public void extractedPackedFieldsAcquireMeaningsOnlyUnderLaterGuards() throws Exception {
    String source = subject("""
      public static boolean hoisted(int tag) {
        int value = tag >>> 3;
        if ((tag & 7) == 1) return value == 2;
        if ((tag & 7) == 2) return value == 2;
        return value == 2;
      }
      public static boolean oneBit(int tag) {
        int value = (tag >>> 1) & 31;
        if ((tag & 1) == 0) return value == 2;
        return value == 2;
      }
      public static boolean directBit(int tag) { return (1 & tag) != 0 && ((tag >>> 1) & 31) == 2; }
      public static boolean changedTag(int tag) {
        int value = (tag >>> 1) & 31;
        tag ^= 1;
        return (tag & 1) == 1 && value == 2;
      }
      public static boolean changedValue(int tag, int raw) {
        int value = tag >>> 3;
        if ((tag & 7) == 1) { value = raw; return value == 2; }
        return false;
      }
      public static boolean severalBits(int tag) {
        int value = tag >>> 3;
        return (tag & 7) != 0 && value == 2;
      }
      """);
    assertTrue(methodSource(source, "hoisted").contains("State.READY"), source);
    assertTrue(methodSource(source, "hoisted").contains("Other.RUN"), source);
    assertTrue(methodSource(source, "hoisted").contains("== 2"), source);
    for (String name : new String[]{"oneBit", "directBit"}) assertTrue(methodSource(source, name).contains("State.READY"), source);
    for (String name : new String[]{"changedTag", "changedValue", "severalBits"}) {
      assertFalse(methodSource(source, name).contains("State.READY"), source);
    }
    recompile();
    try (URLClassLoader original = loader(outRoot()); URLClassLoader result = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      for (String name : new String[]{"hoisted", "oneBit", "directBit", "changedTag", "severalBits"}) {
        for (int tag : new int[]{-1, 0, 1, 2, 4, 5, 16, 17, 18, 19, Integer.MAX_VALUE, Integer.MIN_VALUE}) {
          assertEquals(original.loadClass("sample.AdvancedSubject").getMethod(name, int.class).invoke(null, tag),
            result.loadClass("sample.AdvancedSubject").getMethod(name, int.class).invoke(null, tag), name + ": " + tag);
        }
      }
    }
  }

  @Test
  public void constructorDomainsApplyToSuperThisAndNewCalls() throws IOException {
    String source = subject("""
      int value;
      public AdvancedSubject(int state) { value = state == 2 ? 1 : 0; }
      public AdvancedSubject(long raw) { value = (int)raw; }
      public AdvancedSubject() { this(2); }
      static AdvancedSubject create() { return new AdvancedSubject(2); }
      static AdvancedSubject raw() { return new AdvancedSubject(2L); }
      public static class Child extends AdvancedSubject { public Child() { super(2); } }
      """);
    assertTrue(source.contains("this(State.READY)"), source);
    assertTrue(source.contains("super(State.READY)"), source);
    assertTrue(source.contains("new AdvancedSubject(State.READY)"), source);
    assertTrue(source.contains("new AdvancedSubject(2L)"), source);
    recompile();
  }

  @Test
  public void dependentParametersReturnsAndCallsStayScoped() throws IOException {
    String source = subject("""
      boolean conditional(int tag, int payload) {
        if (tag == 1) return payload == 2;
        if (tag == 2) return payload == 2;
        return payload == 2;
      }
      boolean conditionalSwitch(int tag, int payload) {
        switch (tag) { case 1: return payload == 2; case 2: return payload == 2; default: return payload == 2; }
      }
      boolean shortCircuit(int tag, int payload) { return tag == 1 && payload == 2; }
      boolean mutated(int tag, int payload) { if (tag == 1) { tag++; return payload == 2; } return false; }
      void accept(int tag, int payload) {}
      int produce(int tag) { return 2; }
      int conditionalReturn(int tag) { if (tag == 1) return 2; return 2; }
      void calls(int raw) { accept(1, 2); accept(2, 2); accept(raw, 2); }
      boolean result(int raw) { return produce(1) == 2 && produce(raw) == 2; }
      boolean deferred(int tag) { int value = produce(tag); return tag == 1 && value == 2; }
      """);
    assertTrue(source.contains("accept(1, State.READY)"), source);
    assertTrue(source.contains("accept(2, Other.RUN)"), source);
    assertTrue(source.contains("? State.READY : 2"), source);
    assertTrue(source.contains("produce(1) == State.READY"), source);
    assertTrue(source.contains("== Other.RUN"), source);
    assertTrue(count(source, "== State.READY") >= 5, source);
    assertTrue(count(source, "== 2") >= 3, source);
    recompile();
  }

  @Test
  public void containerContentsAndBoxedValuesPropagateWithoutTypingIndexes() throws IOException {
    String source = subject("""
      java.util.Vector paths;
      java.util.Hashtable table;
      Short boxed;
      boolean path(int index) { return ((Short)paths.elementAt(index)).shortValue() == 2; }
      boolean alias(int index) { java.util.Vector alias = paths; return ((Short)alias.elementAt(index)).shortValue() == 2; }
      boolean unknown(boolean choose, java.util.Vector raw) { return ((Short)(choose ? paths : raw).elementAt(0)).shortValue() == 2; }
      void add() { paths.addElement(new Short((short)2)); table.put("GET", new Integer(2)); }
      boolean lookup() { return ((Integer)table.get("GET")).intValue() == 2; }
      boolean key() { java.util.Enumeration keys = table.keys(); return "POST".equals((String)keys.nextElement()); }
      boolean box() { return boxed.shortValue() == 2; }
      """);
    assertEquals(4, count(source, "== State.READY"), source);
    assertEquals(1, count(source, "== 2"), source);
    assertTrue(source.contains("new Short((short)State.READY)") || source.contains("new Short(State.READY)"), source);
    assertTrue(source.contains("table.put(Method.GET, new Integer(State.READY))"), source);
    assertTrue(source.contains("Method.POST.equals"), source);
    assertFalse(source.contains("elementAt(State."), source);
    recompile();
  }

  @Test
  public void formatsKeepOriginalIntegersAndStringsUseScopedConstants() throws Exception {
    String source = subject("""
      int rgb, argb, fixed;
      String method;
      void assign() { rgb = 16777215; argb = -1; fixed = 384; method = "GET"; }
      boolean color() { int value = rgb; return value == 16777215; }
      boolean text() { return method.equals("POST") || "GET".equals(method); }
      boolean classLiteral() { return method.equals(AdvancedSubject.class); }
      String unrelated() { return "GET"; }
      public static int fixedValue() { return 384; }
      public static int negativeFixed() { return -384; }
      public static int argbValue() { return -1; }
      public static int rgbValue() { return 16777215; }
      static int choose(char value) { return 1; }
      static int choose(int value) { return 2; }
      public static int charOverload() { return choose('A'); }
      static int character(char value) { return value; }
      public static int charCall() { return character('A'); }
      public static int fixedZero() { return 0; }
      public static int fixedMaximum() { return Integer.MAX_VALUE; }
      public static int fixedMinimum() { return Integer.MIN_VALUE; }
      boolean stopped() { return fixed == 0 || fixed < 0; }
      """);
    assertTrue(source.contains("0xFFFFFF"), source);
    assertTrue(source.contains("0xFFFFFFFF"), source);
    assertTrue(source.contains("/* Q8: 1.5 */"), source);
    assertTrue(source.contains("/* Q8: -1.5 */"), source);
    assertFalse(source.contains("/* Q8: 0 */"), source);
    assertTrue(source.contains("return 2147483647;"), source);
    assertTrue(source.contains("return -2147483648;"), source);
    assertTrue(source.contains("method.equals(Method.POST)"), source);
    assertTrue(source.contains("method.equals(AdvancedSubject.class)"), source);
    assertTrue(source.contains("return \"GET\";"), source);
    recompile();
    try (URLClassLoader original = loader(outRoot()); URLClassLoader result = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      for (String name : new String[]{"fixedValue", "negativeFixed", "argbValue", "rgbValue", "charOverload", "charCall", "fixedZero", "fixedMaximum", "fixedMinimum"}) {
        assertEquals(original.loadClass("sample.AdvancedSubject").getMethod(name).invoke(null),
          result.loadClass("sample.AdvancedSubject").getMethod(name).invoke(null), name);
      }
    }
  }

  @Test
  public void boundedIndexesSupportTriplesHeadersAndFieldPlanes() throws Exception {
    String source = subject("""
      int[] triples, planes;
      boolean bounded(int i) { return triples[(i & 255) * 3 + 1] == 2; }
      boolean second(int i) { return triples[(i & 255) * 3 + 2] == 2; }
      boolean guarded(int i) { if (i >= 0 && i < 16) return triples[i * 3 + 1] == 2; return false; }
      boolean overflow(int i) { return triples[i * 3 + 1] == 2; }
      boolean plane(int i) { return planes[102 + (i & 63)] == 2; }
      boolean crossing(int i) { return planes[102 + (i & 255)] == 2; }
      public int planeValue(int input) {
        planes = new int[202];
        for (int i = 0; i < planes.length; i++) planes[i] = i;
        return planes[102 + (input & 63)];
      }
      """);
    assertEquals(2, count(source, "== State.READY"), source);
    assertEquals(2, count(source, "== Other.RUN"), source);
    assertEquals(2, count(source, "== 2"), source);
    assertTrue(source.contains("Fields.OTHER * 101"), source);
    recompile();
    try (URLClassLoader original = loader(outRoot()); URLClassLoader result = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      Class<?> before = original.loadClass("sample.AdvancedSubject"), after = result.loadClass("sample.AdvancedSubject");
      Object originalInstance = before.getConstructor().newInstance(), resultInstance = after.getConstructor().newInstance();
      for (int input : new int[]{Integer.MIN_VALUE, -1, 0, 1, 63, 64, Integer.MAX_VALUE}) {
        assertEquals(before.getMethod("planeValue", int.class).invoke(originalInstance, input),
          after.getMethod("planeValue", int.class).invoke(resultInstance, input), "plane input " + input);
      }
    }
  }

  @Test
  public void loopCountersEstablishRecordAlignmentWithoutGuessingAcrossWritesOrWraps() throws Exception {
    String source = subject("""
      int[] loopPairs, loopTriples;
      int pairs() {
        int sum = 0;
        for (int i = 0; i < loopPairs.length; i += 2) if (loopPairs[i + 1] == 2) sum++;
        return sum;
      }
      int triples() {
        int sum = 0;
        for (int i = 0; i < loopTriples.length; i += 3) {
          int first = loopTriples[i], second = loopTriples[i + 1];
          if (second == 2) sum += first;
        }
        return sum;
      }
      int conditionalAccess(boolean read) {
        int sum = 0;
        for (int i = 0; i < loopTriples.length; i += 3) if (read && loopTriples[i + 1] == 2) sum++;
        return sum;
      }
      int changedCounter() {
        int sum = 0;
        for (int i = 0; i < loopPairs.length; i += 2) { i++; if (loopPairs[i + 1] == 2) sum++; }
        return sum;
      }
      int unknownStart(int start) {
        int sum = 0;
        for (int i = start; i < loopPairs.length; i += 2) if (loopPairs[i + 1] == 2) sum++;
        return sum;
      }
      public static int loopResult() {
        AdvancedSubject subject = new AdvancedSubject();
        subject.loopPairs = new int[]{1, 2, 2, 1};
        subject.loopTriples = new int[]{1, 2, 0, 2, 2, 0};
        return subject.pairs() + subject.triples();
      }
      """);
    assertTrue(methodSource(source, "pairs").contains("Fields.OTHER"), source);
    assertTrue(methodSource(source, "triples").contains("Other.RUN"), source);
    for (String name : new String[]{"conditionalAccess", "changedCounter", "unknownStart"}) {
      assertFalse(methodSource(source, name).contains("Fields.OTHER"), source);
      assertFalse(methodSource(source, name).contains("Other.RUN"), source);
    }
    assertTrue(methodSource(source, "triples").contains("+= 3"), source);
    recompile();
    try (URLClassLoader original = loader(outRoot()); URLClassLoader result = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      assertEquals(original.loadClass("sample.AdvancedSubject").getMethod("loopResult").invoke(null),
        result.loadClass("sample.AdvancedSubject").getMethod("loopResult").invoke(null));
    }
  }

  @Test
  public void tableColumnContractsResolvePerCallAndPreserveAmbiguity() throws IOException {
    String source = subject("""
      byte[] tableA, tableB;
      int lookup(byte[] table, int key) {
        for (int i = 0; i < table.length; i += 2) if (table[i] == key) return table[i + 1];
        return -1;
      }
      boolean first() { return lookup(tableA, 1) == 2; }
      boolean second() { return lookup(tableB, 1) == 2; }
      boolean mixed(boolean choose) { return lookup(choose ? tableA : tableB, 1) == 2; }
      boolean unknown(boolean choose, byte[] raw) { return lookup(choose ? tableA : raw, 1) == 2; }
      int[][] rowsA, rowsB, rowDefaults, uniformRows, refinedRows;
      int nestedLookup(int[][] table, int key) {
        for (int i = 0; i < table.length; i++) if (table[i][0] == key) return table[i][1];
        return -1;
      }
      boolean nestedFirst() { return nestedLookup(rowsA, 1) == 2; }
      boolean nestedSecond() { return nestedLookup(rowsB, 1) == 2; }
      boolean nestedMixed(boolean choose) { return nestedLookup(choose ? rowsA : rowsB, 1) == 2; }
      boolean nestedUnknown(int[][] raw) { return nestedLookup(raw, 1) == 2; }
      boolean rowDefault() { return nestedLookup(rowDefaults, 1) == 2; }
      boolean uniformDefault() { return nestedLookup(uniformRows, 1) == 2; }
      boolean refinedDefault() { return nestedLookup(refinedRows, 1) == 2; }
      """);
    assertTrue(methodSource(source, "first").contains("lookup(this.tableA, Family.A) == State.READY"), source);
    assertTrue(methodSource(source, "second").contains("lookup(this.tableB, Family.A) == Other.RUN"), source);
    assertTrue(methodSource(source, "mixed").contains("== 2"), source);
    assertTrue(methodSource(source, "unknown").contains(", 1) == 2"), source);
    assertFalse(methodSource(source, "lookup").contains("State."), source);
    assertTrue(methodSource(source, "nestedFirst").contains("Family.A) == State.READY"), source);
    assertTrue(methodSource(source, "nestedSecond").contains("Family.A) == Other.RUN"), source);
    for (String name : new String[]{"nestedMixed", "nestedUnknown", "rowDefault"}) {
      assertTrue(methodSource(source, name).contains("== 2"), source);
    }
    assertTrue(methodSource(source, "uniformDefault").contains("== State.READY"), source);
    assertTrue(methodSource(source, "refinedDefault").contains("== State.READY"), source);
    assertFalse(methodSource(source, "nestedLookup").contains("State."), source);
    recompile();
  }

  @Test
  public void negativeAndDefaultCasesRequireTheirActualGuards() throws IOException {
    String source = subject("""
      boolean negative(int tag, int payload) { if (tag == 1) return payload == 2; return payload == 2; }
      boolean otherwise(int tag, int payload) { if (tag == 1 || tag == 2) return payload == 2; return payload == 2; }
      boolean defaultSwitch(int tag, int payload) { switch (tag) { case 1: return false; case 2: return false; default: return payload == 2; } }
      boolean wideSelector(long tag, int payload) { return tag > 0L && payload == 2; }
      void acceptNegative(int tag, int payload) {}
      void negativeCalls(int raw) {
        acceptNegative(99, 2);
        acceptNegative(raw, 2);
        if (raw != 1) acceptNegative(raw, 2);
      }
      """);
    assertTrue(methodSource(source, "negative").contains("Other.RUN"), source);
    assertTrue(methodSource(source, "otherwise").contains("Other.RUN"), source);
    assertTrue(methodSource(source, "defaultSwitch").contains("Other.RUN"), source);
    assertFalse(methodSource(source, "wideSelector").contains("Other.RUN"), source);
    assertEquals(2, count(methodSource(source, "negativeCalls"), "Other.RUN"), source);
    assertTrue(methodSource(source, "negativeCalls").contains(", 2);"), source);
    recompile();
  }

  @Test
  public void recordIndexesKeepAlignmentThroughCopiesAndAgreeingDefinitions() throws Exception {
    String source = subject("""
      static int[] records4, loopTriples;
      public static int copied(int index) { int base = index << 2; int copy = base; return records4[copy + 1] == 2 ? 1 : 0; }
      public static int agreeing(boolean choose, int a, int b) {
        int base;
        if (choose) base = a << 2; else base = b * 4;
        return records4[base + 1] == 2 ? 1 : 0;
      }
      public static int conflicting(boolean choose, int a) {
        int base = choose ? a << 2 : a;
        return records4[base + 1] == 2 ? 1 : 0;
      }
      public static int overwritten(int a, int raw) { int base = a << 2; base = raw; return records4[base + 1] == 2 ? 1 : 0; }
      public static int parameterBeforeWrite(int base) { int value = records4[base + 1]; base = 0; return value == 2 ? 1 : 0; }
      public static int guarded(int a) { int small = a & 255; int base = small * 3; return loopTriples[base + 1] == 2 ? 1 : 0; }
      public static int overflow(int a) { int base = a * 3; return loopTriples[base + 1] == 2 ? 1 : 0; }
      public static int updated(int a) { int base = a << 2; base++; return records4[base + 1] == 2 ? 1 : 0; }
      public static int cyclic(int count) {
        int base = 0;
        while (count-- > 0) base += 3;
        return records4[base + 1] == 2 ? 1 : 0;
      }
      public static int runIndexes(int a) {
        records4 = new int[]{1, 2, 0, 0, 2, 1, 0, 0};
        loopTriples = new int[]{1, 2, 0, 2, 1, 0};
        return copied(a) + agreeing(true, a, 0) + agreeing(false, 0, a);
      }
      """);
    for (String name : new String[]{"copied", "agreeing", "guarded"}) {
      assertTrue(methodSource(source, name).contains("Fields.OTHER"), source);
      assertTrue(methodSource(source, name).contains("Other.RUN"), source);
    }
    for (String name : new String[]{"conflicting", "overwritten", "parameterBeforeWrite", "overflow", "updated", "cyclic"}) {
      assertFalse(methodSource(source, name).contains("Fields.OTHER"), source);
      assertFalse(methodSource(source, name).contains("Other.RUN"), source);
    }
    recompile();
    try (URLClassLoader original = loader(outRoot()); URLClassLoader result = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      for (int index : new int[]{0, 1, 1 << 30, Integer.MIN_VALUE}) {
        assertEquals(original.loadClass("sample.AdvancedSubject").getMethod("runIndexes", int.class).invoke(null, index),
          result.loadClass("sample.AdvancedSubject").getMethod("runIndexes", int.class).invoke(null, index));
      }
    }
  }

  @Test
  public void signedFlagOperandsKeepTheirBitsAndPromotedExpressionType() throws Exception {
    String source = subject("""
      static int flags;
      public static int signedByte() { return flags & -128; }
      public static int signedShort() { return flags & -32768; }
      public static int combinedByte() { return flags & -127; }
      public static int unsignedByte() { return flags & 128; }
      public static int unknownBits() { return flags & -126; }
      public static long wide() { return ((long)flags) & -128L; }
      static int pick(byte value) { return 1; }
      static int pick(short value) { return 2; }
      static int pick(int value) { return 3; }
      public static int overloaded() { return pick(flags & -128); }
      public static long runFlags(int value) {
        flags = value;
        return signedByte() + signedShort() + combinedByte() + unsignedByte() + unknownBits() + wide() + overloaded();
      }
      """);
    assertTrue(methodSource(source, "signedByte").contains("(byte)(ByteFlags.HIGH)"), source);
    assertTrue(methodSource(source, "signedShort").contains("(short)(ByteFlags.WIDE)"), source);
    assertTrue(methodSource(source, "combinedByte").contains("(byte)("), source);
    assertFalse(methodSource(source, "unsignedByte").contains("(byte)"), source);
    assertFalse(methodSource(source, "unknownBits").contains("(byte)"), source);
    assertFalse(methodSource(source, "wide").contains("(byte)"), source);
    recompile();
    try (URLClassLoader original = loader(outRoot()); URLClassLoader result = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      for (int value : new int[]{0, 1, 127, 128, 129, -128, -127, -1, 32768, Integer.MIN_VALUE, Integer.MAX_VALUE}) {
        assertEquals(original.loadClass("sample.AdvancedSubject").getMethod("runFlags", int.class).invoke(null, value),
          result.loadClass("sample.AdvancedSubject").getMethod("runFlags", int.class).invoke(null, value));
      }
    }
  }

  private static String methodSource(String source, String name) {
    int start = source.indexOf(" " + name + "(");
    assertTrue(start >= 0, source);
    int brace = source.indexOf('{', start), end = brace + 1, depth = 1;
    while (depth != 0) {
      char c = source.charAt(end++);
      if (c == '{') depth++;
      else if (c == '}') depth--;
    }
    return source.substring(start, end);
  }

  private String subject(String body) throws IOException {
    return compileDecompileAndRead("sample/AdvancedSubject.java", "package sample; public class AdvancedSubject {\n" + body + "\n}");
  }

  private static URLClassLoader loader(Path directory) throws IOException {
    return new URLClassLoader(new URL[]{directory.toUri().toURL()}, ClassLoader.getPlatformClassLoader());
  }

  private static int count(String text, String needle) { return (text.length() - text.replace(needle, "").length()) / needle.length(); }
}
