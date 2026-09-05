package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import javax.tools.ToolProvider;
import java.io.IOException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertSame;

public class LegacyCapturedCatchRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testCapturedExceptionRemainsFinalForLegacySource() throws Exception {
    Path source = writeSource("pkg/CapturedCatch.java", """
      package pkg;
      import java.io.IOException;
      public class CapturedCatch {
        public IOException observed;
        public Runnable make(IOException failure) {
          try {
            throw failure;
          } catch (final IOException caught) {
            return new Runnable() {
              public void run() { observed = caught; }
            };
          }
        }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    Path anonymous = outRoot().resolve("pkg/CapturedCatch$1.class");
    Files.write(anonymous, ClassFileTestUtil.removeClassAttribute(Files.readAllBytes(anonymous), "EnclosingMethod"));
    for (Path file : java.util.List.of(anonymous, outRoot().resolve("pkg/CapturedCatch.class"))) {
      byte[] bytes = Files.readAllBytes(file);
      ClassFileTestUtil.putU2(bytes, 6, 48);
      Files.write(file, bytes);
    }
    assertBehavior(outRoot());
    decompileDirectory(outRoot(), "pkg/CapturedCatch.java");

    // Java 8 accepts effectively-final captures and would hide this regression.
    // Java 7 has the same explicit-final requirement as the older source targets.
    Path recompiled = fixture.getTempDir().resolve("recompiled-out");
    Files.createDirectories(recompiled);
    String[] arguments = java.util.stream.Stream.concat(
      java.util.stream.Stream.of("-source", "7", "-target", "7", "-d", recompiled.toString()),
      listJavaSources(fixture.getTargetDir()).stream().map(Path::toString)
    ).toArray(String[]::new);
    assertEquals(0, ToolProvider.getSystemJavaCompiler().run(null, null, null, arguments));
    assertBehavior(recompiled);
  }

  private static void assertBehavior(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("pkg.CapturedCatch");
      Object instance = type.getConstructor().newInstance();
      IOException first = new IOException("first");
      IOException second = new IOException("second");
      Runnable firstTask = (Runnable)type.getMethod("make", IOException.class).invoke(instance, first);
      Runnable secondTask = (Runnable)type.getMethod("make", IOException.class).invoke(instance, second);
      firstTask.run();
      assertSame(first, type.getField("observed").get(instance));
      secondTask.run();
      assertSame(second, type.getField("observed").get(instance));
      firstTask.run();
      assertSame(first, type.getField("observed").get(instance));
    }
  }
}
