package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;

public class SyntheticMemberRetentionTest extends DecompileRegressionTestBase {
  @Test
  public void testExtractedInitializersRetainTransitiveHelpers() throws Exception {
    Path source = writeSource("pkg/InitializerCalls.java", """
      package pkg;
      public class InitializerCalls {
        public static int calls;
        public static int shared = first();
        public int instance = instanceOnly();
        private static int first() { return second() + 1; }
        private static int second() { calls++; return 7; }
        private static int instanceOnly() { calls += 10; return 8; }
        private static int unused() { return -1; }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    Path file = outRoot().resolve("pkg/InitializerCalls.class");
    byte[] bytes = Files.readAllBytes(file);
    for (String method : new String[]{"first", "second", "instanceOnly", "unused"}) {
      bytes = ClassFileTestUtil.addMemberFlags(bytes, true, method, CodeConstants.ACC_SYNTHETIC);
    }
    bytes = ClassFileTestUtil.addMemberFlags(bytes, false, "calls", CodeConstants.ACC_SYNTHETIC);
    Files.write(file, bytes);
    String output = decompileDirectory(outRoot(), "pkg/InitializerCalls.java");
    assertContains(output, "shared = first()", "instance = instanceOnly()");
    assertNotContains(output, "unused()");
    recompile();
    try (BehaviorComparison behavior = compareBehavior()) {
      behavior.assertSame("initializer values and calls", new Object[]{8, 8, 11}, loader -> {
        Class<?> cls = loader.loadClass("pkg.InitializerCalls");
        Object instance = cls.getConstructor().newInstance();
        return new Object[]{cls.getField("shared").get(null), cls.getField("instance").get(instance), cls.getField("calls").get(null)};
      });
    }
  }
}
