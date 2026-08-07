package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;

import static org.junit.jupiter.api.Assertions.assertTrue;

public class ArrayInitializerFormattingRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[]{IFernflowerPreferences.PREFERRED_LINE_LENGTH, "72"};
  }

  @Test
  public void testLongArrayInitializersPackMultipleElementsPerLine() throws IOException {
    String content = compileDecompileAndRead("pkg/ArrayInitializerFormatting.java", """
      package pkg;

      class V {
        static int A;
        static int B;
        static int C;
        static int D;
        static int E;
        static int F;
        static int G;
        static int H;
        static int I;
        static int J;
        static int K;
        static int L;
        static int M;
        static int N;
        static int O;
        static int P;
        static int Q;
        static int R;
      }

      public class ArrayInitializerFormatting {
        public int[] constants() {
          return new int[]{
            100, 101, 102, 103, 104, 105, 106, 107, 108, 109,
            110, 111, 112, 113, 114, 115, 116, 117, 118, 119
          };
        }

        public int[] fields() {
          return new int[]{
            V.A, V.B, V.C, V.D, V.E, V.F, V.G, V.H, V.I,
            V.J, V.K, V.L, V.M, V.N, V.O, V.P, V.Q, V.R
          };
        }

        public int[][] rows() {
          return new int[][]{
            {
              1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
              13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24
            },
            {
              25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36,
              37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48
            }
          };
        }
      }
      """);

    assertTrue(content.contains("100, 101, 102"), content);
    assertTrue(content.contains("V.A, V.B"), content);
    assertTrue(content.contains("1, 2, 3"), content);
    assertTrue(content.contains("25, 26, 27"), content);
    for (String line : content.split("\n")) {
      assertTrue(line.length() <= 72, () -> "Line exceeds preferred length: " + line);
    }

    recompile();
  }
}
