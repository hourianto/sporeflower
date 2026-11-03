package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

public class SemanticPackedFieldsRegressionTest extends SemanticTestBase {
  public SemanticPackedFieldsRegressionTest() {
    super("packed-fields", "PackedProbe");
  }

  @Test
  public void namedSlicesPreserveConditionalDomainsAndSeparateReusedLocals() throws Exception {
    String source = subject("""
      static int[][] data = new int[1][1];
      public static int decode(int word) {
        data[0][0] = word;
        int raw = data[0][0];
        int value = (raw & 30) >> 1;
        if ((raw & 1) == 0) {
          raw = value;
          return (raw == 2 ? 1 : 0) | (value == 2 ? 2 : 0);
        } else {
          int g = (96 & raw) >> 5;
          int s = (raw >> 7) & 3;
          raw = (raw & 65024) >> 9;
          return (value == 3 ? 4 : 0) | (g == 2 ? 8 : 0) | (s == 1 ? 16 : 0) | (raw << 5);
        }
      }
      public static int untyped(int word) { return (word & 30) >> 1; }
      public static int wrongMask(int word) { data[0][0] = word; return (data[0][0] & 14) >> 1; }
      public static int encode(int value, int g, int s, int progress) {
        data[0][0] = 1 | (value & 15) << 1 | g << 5 | s << 7 | progress << 9;
        return data[0][0];
      }
      public static int encodeMasked(int kind, int value) {
        data[0][0] = (1 & kind) | (15 & value) << 1;
        return data[0][0];
      }
      """);
    assertContains(source, "Packet.SUBTYPE_MASK", "Packet.SUBTYPE_SHIFT", "Packet.KIND_MASK", "Packet.GROUP_MASK",
      "Packet.GROUP_SHIFT", "Packet.STAGE_VALUE_MASK", "Packet.PROGRESS_MASK", "Packet.PROGRESS_SHIFT",
      "PacketKind.PLAIN", "ItemCode.ITEM", "UnitCode.UNIT", "Group.GROUP", "Stage.STAGE",
      "int packedPacket", "int subtype", "int group", "int stage", "int progress");
    assertNotContains(source, "subtype = subtype;");
    assertNotContains(methodSource(source, "untyped"), "Packet.");
    assertNotContains(methodSource(source, "wrongMask"), "SUBTYPE_");
    assertContains(methodSource(source, "encode"), "Packet.SUBTYPE_VALUE_MASK", "Packet.SUBTYPE_SHIFT",
      "Packet.GROUP_SHIFT", "Packet.STAGE_SHIFT", "Packet.PROGRESS_SHIFT");
    assertContains(methodSource(source, "encodeMasked"), "Packet.KIND_MASK", "Packet.SUBTYPE_VALUE_MASK", "Packet.SUBTYPE_SHIFT");
    recompile();
    try (BehaviorComparison behavior = compareBehavior()) {
      for (int word = 0; word <= 65535; word++) {
        int input = word;
        behavior.assertSame("word=" + word, loader ->
          loader.loadClass("sample.PackedProbe").getMethod("decode", int.class).invoke(null, input));
      }
      for (int word : new int[]{-1, Integer.MIN_VALUE, Integer.MAX_VALUE}) {
        behavior.assertSame("word=" + word, loader ->
          loader.loadClass("sample.PackedProbe").getMethod("decode", int.class).invoke(null, word));
      }
      for (int word : new int[]{0, 1, -1, 127, Integer.MIN_VALUE, Integer.MAX_VALUE}) {
        behavior.assertSame("encoding=" + word, loader -> loader.loadClass("sample.PackedProbe")
          .getMethod("encode", int.class, int.class, int.class, int.class).invoke(null, word, word, word, word));
      }
      for (int word : new int[]{0, 1, -1, 127, Integer.MIN_VALUE, Integer.MAX_VALUE}) {
        behavior.assertSame("masked encoding=" + word, loader -> loader.loadClass("sample.PackedProbe")
          .getMethod("encodeMasked", int.class, int.class).invoke(null, word, word));
      }
    }
  }

  @Test
  public void namedMasksPreserveIntAndLongWidthsAndSignedExtraction() throws Exception {
    String source = subject("""
      static int narrow;
      static long lowWord, wide;
      public static int top(int word) { narrow = word; return (narrow & 0x80000000) >>> 31; }
      public static long low(long word) { lowWord = word; return lowWord & 0xffffffffL; }
      public static long encodeLow(long word) { lowWord = word & 0xffffffffL; return lowWord; }
      public static long normalizeLow(long word) { lowWord = word; return lowWord = (lowWord >>> 0) & 0xffffffffL; }
      public static long updateLow(long word) { lowWord = word; return lowWord &= 0xffffffffL; }
      public static long high(long word) { wide = word; return (wide >>> 63) & 1L; }
      public static long group(long word) { wide = word; return (wide >>> 32) & 65535L; }
      public static long signed(long word) { wide = word; long value = (wide << 56) >> 56; return value * value + value; }
      public static int wrappedShift(int word) { narrow = word; return (narrow & 0x80000000) >>> 63; }
      """);
    assertTrue(source.contains("(int)Narrow.TOP_MASK") || source.contains("(int)(Narrow.TOP_MASK)"), source);
    assertContains(source, "LowWord.PAYLOAD_MASK", "Wide.TOP_SHIFT", "Wide.TOP_VALUE_MASK", "Wide.GROUP_SHIFT", "long delta");
    assertContains(methodSource(source, "encodeLow"), "LowWord.PAYLOAD_MASK");
    assertContains(methodSource(source, "normalizeLow"), "LowWord.PAYLOAD_MASK");
    assertContains(methodSource(source, "updateLow"), "&= LowWord.PAYLOAD_MASK");
    recompile();
    assertSameIntResults(new String[]{"top", "wrappedShift"}, 0, 1, -1, Integer.MIN_VALUE, Integer.MAX_VALUE);
    Object[][] words = java.util.stream.LongStream.of(0, 1, -1, 127, 128, 255, 0x80000000L, 0xffffffffL,
      0x123456789abcdef0L, Long.MIN_VALUE, Long.MAX_VALUE).mapToObj(word -> new Object[]{word}).toArray(Object[][]::new);
    assertSameResults(new String[]{"low", "encodeLow", "normalizeLow", "updateLow", "high", "group", "signed"},
      new Class<?>[]{long.class}, words);
  }

  @Test
  public void fieldNamesDoNotShadowStaticFieldsOrDescribeOpaqueDefinitions() throws Exception {
    String source = subject("""
      static int[][] data = new int[1][1];
      static int group;
      static int shadow;
      public static int collision(int word) {
        data[0][0] = word;
        group = 99;
        int value = (data[0][0] & 96) >> 5;
        return value + group;
      }
      public static int opaque(int word, boolean replace, int other) {
        data[0][0] = word;
        int value = (data[0][0] & 30) >> 1;
        if (replace) value = other;
        return value * value + value;
      }
      public static int classCollision(int word) {
        data[0][0] = shadow = word;
        int value = shadow & 15;
        return value * value + value + ((data[0][0] & 30) >> 1);
      }
      """);
    assertContains(methodSource(source, "collision"), "int group", "PackedProbe.group");
    assertNotContains(methodSource(source, "opaque"), "int subtype");
    assertContains(methodSource(source, "classCollision"), "int Packet");
    recompile();
    assertSameIntResults(new String[]{"classCollision", "collision"}, 0, 1, 31, 97, -1);
  }

  @Test
  public void packedProducerLayoutsSurviveOtherConsumerInterpretations() throws Exception {
    String source = subject("""
      static short[] references = new short[1], otherReferences = new short[1], itemIds = new short[1];
      static int[] widths = new int[32768];
      static int consume(int id) { return id + 1; }
      public static int branch(int input) {
        references[0] = (short)input;
        short word;
        if ((word = references[0]) < 0) return word & 32767;
        return consume(word);
      }
      public static int indexed(int input) {
        references[0] = (short)input;
        short word = references[0];
        if (word < 0) return word & 32767;
        return widths[word];
      }
      public static int decoded(int input) {
        references[0] = (short)input;
        short word = references[0];
        if (word < 0) return (word & 32767) == 2 ? 1 : 0;
        return consume(word);
      }
      public static int competing(int input) {
        short word = input < 0 ? references[0] : otherReferences[0];
        return word & 32767;
      }
      public static int mixed(int input) {
        short word = input < 0 ? references[0] : itemIds[0];
        return word & 32767;
      }
      public static int unknown(int input) {
        short word = references[0];
        if (input > 0) word = (short)input;
        return word & 32767;
      }
      public static int wrongMask(int input) { return references[0] & 127; }
      """);
    assertMethodsContain(source, new String[]{"branch", "indexed", "decoded"}, "Reference.REFERENCE_ID_MASK", "< 0");
    assertContains(methodSource(source, "decoded"), "ItemCode.ITEM");
    assertMethodsDoNotContain(source, new String[]{"competing", "mixed", "unknown", "wrongMask"}, "REFERENCE_ID_MASK");
    recompile();
    assertSameIntResults(new String[]{"branch", "indexed", "decoded", "competing", "mixed", "unknown", "wrongMask"},
      0, 1, 2, 32767, 32768, 32770, 65535, -1, Integer.MIN_VALUE, Integer.MAX_VALUE);
  }

  @Test
  public void debugLocalNamesTakePrecedenceOverLayoutNames() throws Exception {
    var input = writeSource("sample/PackedProbe.java", """
      package sample;
      public class PackedProbe {
        static int[][] data = new int[1][1];
        public static int decode(int group) {
          data[0][0] = group;
          int originalProgress = (data[0][0] & 65024) >> 9;
          return originalProgress * originalProgress + originalProgress;
        }
      }
      """);
    compileJava8WithDebug(input, outRoot());
    String source = decompileDirectory(outRoot(), "sample/PackedProbe.java");
    assertContains(source, "int group", "int originalProgress", "Packet.PROGRESS_MASK");
    recompile();
  }
}
