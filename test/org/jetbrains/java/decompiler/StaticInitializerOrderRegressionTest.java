package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.assertAll;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class StaticInitializerOrderRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void preservesStaticAssignmentAfterConstructorSideEffect() throws IOException {
    String content = compileDecompileAndRead("pkg/TestStaticInitializerOrder.java", """
      package pkg;

      import java.util.Vector;

      public final class TestStaticInitializerOrder {
        private static Object cache;
        private static int master;
        private static int effective;
        private static boolean wav;
        private static int current;
        private static int loop;
        private static byte[] preload;
        private static String[] supported;
        private static boolean enabled;

        private TestStaticInitializerOrder() {
          supported = new String[]{"audio/x-wav"};
          wav = true;
        }

        public static String[] supported() {
          return supported;
        }

        static {
          cache = new Object();
          new Vector();
          master = effective = 100;
          wav = false;
          current = -1;
          loop = 1;
          preload = null;
          new TestStaticInitializerOrder();
          supported = null;
          enabled = true;
        }
      }
      """);

    assertFalse(content.contains("private static String[] supported = null;"), content);

    int constructorCall = content.indexOf("new TestStaticInitializerOrder();");
    int clearAfterProbe = content.indexOf("supported = null;");
    assertTrue(constructorCall >= 0, content);
    assertTrue(clearAfterProbe > constructorCall, content);
  }

  @Test
  public void preservesLiftedStaticInitializerDependencyOrder() throws IOException {
    String content = compileDecompileAndRead("pkg/TestStaticFieldInitializerDependencyOrder.java", """
      package pkg;

      public final class TestStaticFieldInitializerDependencyOrder {
        static final class BufferOwner {
          private static byte[] buffer;
          private static byte level;
          private static byte maxLevel;
          private static int bufferLength;

          static {
            maxLevel = 1;
            bufferLength = 4096;
            level = maxLevel;
            buffer = new byte[bufferLength];
          }
        }

        static final class DependentBuffer {
          private static byte[] snapshot;
          private static int snapshotLength;

          static {
            snapshotLength = BufferOwner.bufferLength;
            snapshot = new byte[snapshotLength];
          }
        }

        static final class IndexSelection {
          private static int[] activeIndexes;
          private static final int[] defaultIndexes;

          static {
            defaultIndexes = new int[]{0, 1, 2, 3, 4};
            activeIndexes = defaultIndexes;
          }
        }
      }
      """);

    assertAll(
      () -> assertPatternBefore(content, "bufferLength = 4096", "buffer = new byte"),
      () -> assertPatternBefore(content, "maxLevel = 1", "level = .*maxLevel"),
      () -> assertPatternBefore(content, "snapshotLength = .*BufferOwner\\.bufferLength", "snapshot = new byte"),
      () -> assertPatternBefore(content, "defaultIndexes = new int\\[\\]\\{0, 1, 2, 3, 4\\}", "activeIndexes = .*defaultIndexes")
    );
  }

  private static void assertPatternBefore(String content, String earlierRegex, String laterRegex) {
    int earlier = find(content, earlierRegex);
    int later = find(content, laterRegex);
    assertTrue(earlier >= 0, "Missing earlier pattern /" + earlierRegex + "/ in:\n" + content);
    assertTrue(later >= 0, "Missing later pattern /" + laterRegex + "/ in:\n" + content);
    assertTrue(earlier < later, "Expected /" + earlierRegex + "/ before /" + laterRegex + "/ in:\n" + content);
  }

  private static int find(String content, String regex) {
    Matcher matcher = Pattern.compile(regex).matcher(content);
    return matcher.find() ? matcher.start() : -1;
  }
}
