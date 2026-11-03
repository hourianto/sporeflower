package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;

public class NumericSourceRecoveryTest extends DecompileRegressionTestBase {
  @Test
  public void testBytecharPlaceholderIsNotEmittedInJavaOutput() throws IOException {
    String content = compileDecompileAndRead("pkg/TestBytecharCastRender.java", """
      package pkg;

      public class TestBytecharCastRender {
        public static void mark(Object grid, int x, int y) {
          if (((byte[]) ((Object[]) grid)[x])[y] > 0) {
            ((byte[]) ((Object[]) grid)[x])[y] = (byte) (((byte[]) ((Object[]) grid)[x])[y] | 4);
          }
        }
      }
      """);

    assertNotContains(content,
      "<bytechar>",
      "<shortchar>");
    recompile();
  }

  @Test
  public void testIntegerNumeratorsStayWideUntilExplicitArrayStoreNarrowing() throws IOException {
    String content = compileDecompileAndRead(
      "pkg/TestIntNumeratorNoPrematureNarrow.java",
      """
        package pkg;

        public class TestIntNumeratorNoPrematureNarrow {
          static byte mission;

          public static void shortScale(short[] out, int distance, int h) {
            int numerator = 8000;
            if (distance > 0) {
              numerator = mission == -5 ? 40000 : 20000;
            }
            out[7] = (short)(numerator / ((distance << 1) + Math.min(h, 208) + 10));
          }

          public static void byteScale(byte[] out, int distance) {
            int numerator = 12;
            if (distance > 0) {
              numerator = mission == -5 ? 1000 : 200;
            }
            out[1] = (byte)(numerator / (distance + 3));
          }

          public static void charScale(char[] out, int distance) {
            int numerator = 65;
            if (distance > 0) {
              numerator = mission == -5 ? 70000 : 40000;
            }
            out[2] = (char)(numerator / (distance + 3));
          }
        }
        """
    );

    assertNotContains(content,
      "short var3 = 8000",
      "byte var3 = 12",
      "char var3 = 65",
      "= (short)(mission == -5 ? 40000 : 20000)",
      "= (byte)(mission == -5 ? 1000 : 200)",
      "= (char)(mission == -5 ? 70000 : 40000)");

    assertContains(content,
      "int var3 = 8000;",
      "var3 = mission == -5 ? 40000 : 20000;",
      "int var2 = 12;",
      "var2 = mission == -5 ? 1000 : 200;",
      "int var2 = 65;",
      "var2 = mission == -5 ? 70000 : 40000;");

    recompile();
  }

  @Test
  public void nestedDefinitionKeepsItsWideNumericType() throws Exception {
    compileDecompileAndRead("pkg/TestNestedNumericTernary.java", """
      package pkg;
      public class TestNestedNumericTernary {
        public static long scale(String text) {
          long value;
          return ((value = Long.parseLong(text)) == 0L ? 1L : value) * 24L * 3600L * 1000L;
        }
      }
      """);
    recompile();
    try (BehaviorComparison behavior = compareBehavior()) {
      for (long value : new long[]{0, 1, -1, Integer.MAX_VALUE + 1L, Long.MAX_VALUE}) {
        behavior.assertSameStaticCall("pkg.TestNestedNumericTernary", "scale", new Class<?>[]{String.class}, Long.toString(value));
      }
    }
  }

  @Test
  public void testSwitchCaseSlotReuseDoesNotNarrowIntAssignmentToByte() throws IOException {
    compileDecompileAndRead("Repro.java", """
      import java.util.Vector;

      public class Repro {
        static byte[] field_234 = new byte[32];

        public static int test(Vector a, int var1) {
          int var7 = var1;
          Object[] var3 = (Object[])a.elementAt(var7);
          int[] var4;
          int var10000;
          if ((var4 = (int[])var3[0])[2] + 1 < field_234.length) {
            byte var17 = field_234[++var4[2]];
            var3[0] = var4;
            a.setElementAt(var3, var7);
            var10000 = var17;
          } else {
            var10000 = 15;
          }

          byte var8 = (byte)var10000;
          byte var18 = var8;
          int var9 = var1;
          switch (var18) {
            case 2:
              int var30 = var9;
              int[] var60 = (int[])((Object[])a.elementAt(var30))[0];
              Object var48 = null;
              var10000 = var60[2];
              int var31 = var10000;
              short var38 = 7;
              return var38 + var31 - 1;
            default:
              return 0;
          }
        }
      }
      """);

    recompile();
  }
}
