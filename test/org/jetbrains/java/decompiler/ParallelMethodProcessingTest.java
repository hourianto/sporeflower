package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.api.Decompiler;
import org.jetbrains.java.decompiler.main.decompiler.DirectoryResultSaver;
import org.jetbrains.java.decompiler.main.extern.IFernflowerLogger;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;

import static org.junit.jupiter.api.Assertions.*;

public class ParallelMethodProcessingTest extends DecompileRegressionTestBase {
  @Test
  void substantialMethodsShareWorkersAndPreserveSerialOutputAndBehavior() throws Exception {
    StringBuilder source = new StringBuilder("public class Workload {\n");
    for (int method = 0; method < 32; method++) {
      source.append("public static int work").append(method).append("(int input) { int result = input;\n");
      source.append("try { for (int step = 0; step < 3; step++) {\n");
      for (int operation = 0; operation < 48; operation++) {
        source.append("result = Integer.rotateLeft(result ^ ").append(method * 48 + operation)
          .append(", 3) + step;\n");
      }
      source.append("} } catch (RuntimeException failure) { return failure.hashCode(); } return result; }\n");
    }
    source.append("}");
    compileJava8NoDebug(writeSource("Workload.java", source.toString()), outRoot());

    Path serial = fixture.getTempDir().resolve("serial");
    Path parallel = fixture.getTempDir().resolve("parallel");
    decompile(serial, false);
    Set<Long> workers = decompile(parallel, true);
    assertTrue(workers.size() > 1, "Expected methods of one class to use multiple workers");
    assertEquals(Files.readString(serial.resolve("Workload.java")), Files.readString(parallel.resolve("Workload.java")));

    Path recompiled = fixture.getTempDir().resolve("recompiled");
    compileJava8NoDebug(parallel.resolve("Workload.java"), recompiled);
    try (URLClassLoader original = loader(outRoot()); URLClassLoader restored = loader(recompiled)) {
      for (int method : new int[]{0, 15, 31}) {
        for (int input : new int[]{0, -1, 42, Integer.MAX_VALUE}) {
          Object expected = original.loadClass("Workload").getMethod("work" + method, int.class).invoke(null, input);
          Object actual = restored.loadClass("Workload").getMethod("work" + method, int.class).invoke(null, input);
          assertEquals(expected, actual);
        }
      }
    }
  }

  private Set<Long> decompile(Path output, boolean parallel) {
    Set<Long> workers = ConcurrentHashMap.newKeySet();
    Decompiler.builder().inputs(outRoot().toFile()).output(new DirectoryResultSaver(output.toFile()))
      .option(IFernflowerPreferences.THREADS, "4")
      .option(IFernflowerPreferences.PARALLEL_METHODS, parallel ? "1" : "0")
      .logger(new IFernflowerLogger() {
        @Override public void startMethod(String name) { workers.add(Thread.currentThread().getId()); }
        @Override public void writeMessage(String message, Severity severity) { }
        @Override public void writeMessage(String message, Severity severity, Throwable failure) {
          throw new AssertionError(message, failure);
        }
      }).build().decompile();
    return workers;
  }

  private static URLClassLoader loader(Path classes) throws Exception {
    return new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader());
  }
}
