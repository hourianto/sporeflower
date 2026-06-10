package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;

import static org.junit.jupiter.api.Assertions.assertFalse;

public class LabeledControlFlowTargetRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testLoopHeaderJumpAfterGuardDoesNotUseOutOfScopeLabel() throws IOException {
    String content = compileDecompileAndRead("pkg/TestLabeledControlFlowTarget.java", """
      package pkg;

      import java.io.DataInputStream;
      import java.io.InputStream;

      public class TestLabeledControlFlowTarget {
        static int[] ints;
        static short[] shorts;

        public static void test(InputStream input) {
          if (input != null) {
            try {
              DataInputStream data = new DataInputStream(input);
              short tag = data.readShort();
              short length = data.readShort();
              if (tag != 10 && length != 20) {
                return;
              }

              while ((tag = data.readShort()) != -1) {
                length = data.readShort();
                switch (tag) {
                  case 1:
                    ints = new int[tag = data.readShort()];
                    for (int i = 0; i < tag; i++) {
                      ints[i] = data.readInt();
                    }
                    break;
                  case 2:
                    shorts = new short[tag = data.readShort()];
                    for (int i = 0; i < tag; i++) {
                      shorts[i] = data.readShort();
                    }
                    break;
                  default:
                    data.skip(length);
                }
              }
            } catch (Throwable ignored) {
              return;
            } finally {
              try {
                input.close();
              } catch (Throwable ignored) {
              }
            }
          }
        }
      }
      """);
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(content.contains("break label"), content);

    recompile();
  }
}
