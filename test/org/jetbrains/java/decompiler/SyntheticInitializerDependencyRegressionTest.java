package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.junit.jupiter.api.Test;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class SyntheticInitializerDependencyRegressionTest extends DecompileRegressionTestBase {
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
    assertTrue(output.contains("shared = first()"), output);
    assertTrue(output.contains("instance = instanceOnly()"), output);
    org.junit.jupiter.api.Assertions.assertFalse(output.contains("unused()"), output);
    recompile();
    assertEquals(outcome(outRoot()), outcome(fixture.getTempDir().resolve("recompiled-out")));
  }

  private static String outcome(Path classes) throws Exception {
    try (var loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> cls = loader.loadClass("pkg.InitializerCalls");
      Object instance = cls.getConstructor().newInstance();
      return cls.getField("shared").get(null) + ":" + cls.getField("instance").get(instance) + ":" + cls.getField("calls").get(null);
    }
  }
}
