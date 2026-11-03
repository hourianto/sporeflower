package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.util.DataInputFullStream;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.*;

class Tiny2ParameterReuseRegressionTest extends TinyMappingTestBase {
  @Override
  protected String tinyMappings() {
    return """
      tiny\t2\t0\tofficial\tnamed
      c\tWorker\tNamedWorker
      \tm\t(II)I\tunused\ttick
      \t\tp\t1\t\tclockMillis
      \t\tp\t2\t\tlimit
      \tm\t(II)I\tused\tused
      \t\tp\t1\t\tclockMillis
      \t\tp\t2\t\tlimit
      \tm\t(JII)I\twide\twide
      \t\tp\t0\t\tclockMillis
      \t\tp\t2\t\tdeltaMillis
      \t\tp\t3\t\tlimit
      \tm\t(I)I\tcountdown\tcountdown
      \t\tp\t0\t\tremaining
      \tm\t(I)I\tclamp\tclamp
      \t\tp\t0\t\tdeltaMillis
      """;
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void mappedNamesDescribeParametersRatherThanTheirReusedSlots(boolean strict) throws Exception {
    configureOptions(IFernflowerPreferences.MAPPINGS_PATH, mappingDirectory.resolve("mappings.tiny").toString(),
      IFernflowerPreferences.J2ME_STRICT_SLOT_MERGE, strict ? "1" : "0");
    Path input = writeSource("Worker.java", """
      public class Worker {
        public int unused(int clock, int limit) {
          int result = 0;
          for (clock = 0; clock < limit; clock++) result += clock;
          return result;
        }
        public int used(int clock, int limit) {
          int result = clock;
          for (clock = 0; clock < limit; clock++) result += clock;
          for (clock = limit; clock > 0; clock--) result ^= clock;
          return result;
        }
        public static int wide(long clock, int delta, int limit) {
          int result = (int)clock + delta;
          for (delta = 0; delta < limit; delta++) result += delta;
          for (clock = 0; clock < limit; clock++) result ^= (int)clock;
          return result;
        }
        public static int countdown(int count) {
          int result = 0;
          while (count > 0) { result += count; count--; }
          return result;
        }
        public static int clamp(int delta) {
          if (delta < 0) delta = 0;
          return delta * delta + delta;
        }
      }
      """);
    compileJava8NoDebug(input, outRoot());
    // Inspect the input bytecode using the same configured context as decompilation.
    fixture.getDecompiler();
    assertParameterStores(outRoot().resolve("Worker.class"));
    checkBehavior(outRoot(), "Worker", "unused");
    String source = decompileDirectory(outRoot(), "NamedWorker.java");
    recompile();
    checkBehavior(recompiledRoot(), "NamedWorker", "tick");

    String independent = source.substring(0, source.indexOf("public static int countdown("));
    assertContains(independent, "tick(int clockMillis, int limit)", "wide(long clockMillis, int deltaMillis, int limit)");
    assertFalse(Pattern.compile("\\b(?:clockMillis|deltaMillis)\\s*(?:=(?!=)|\\+\\+|--|[+^-]=)")
      .matcher(independent).find(), "Independent lifetimes must not inherit a parameter's mapped name:\n" + source);
    // A loop that consumes the caller's value and a conditional normalization
    // are connected parameter lifetimes, not unrelated slot reuse.
    assertTrue(source.contains("remaining--") || source.contains("--remaining"), source);
    assertContains(source, "deltaMillis = 0");
    assertFalse(Pattern.compile("\\bint \\w+ = remaining;").matcher(source).find(),
      "Do not introduce a copy for an ordinary parameter countdown:\n" + source);
  }

  private static void assertParameterStores(Path file) throws Exception {
    try (var stream = new DataInputFullStream(Files.readAllBytes(file))) {
      StructClass type = StructClass.create(stream, true);
      for (String name : List.of("unused", "used", "wide")) {
        var method = type.getMethod(name, name.equals("wide") ? "(JII)I" : "(II)I");
        method.expandData(type);
        boolean stored = false;
        for (var instruction : method.getInstructionSequence()) {
          stored |= instruction.opcode == CodeConstants.opc_istore && instruction.operand(0) == (name.equals("wide") ? 2 : 1);
        }
        assertTrue(stored, "Fixture must overwrite the parameter slot in " + name);
      }
    }
  }

  private static void checkBehavior(Path classes, String className, String unusedName) throws Exception {
    try (var loader = loader(classes)) {
      Class<?> type = loader.loadClass(className);
      Object instance = type.getConstructor().newInstance();
      for (int limit : new int[]{-1, 0, 1, 8}) {
        int sum = 0;
        for (int i = 0; i < limit; i++) sum += i;
        assertEquals(sum, type.getMethod(unusedName, int.class, int.class).invoke(instance, 31, limit));
        int used = 31 + sum;
        for (int i = limit; i > 0; i--) used ^= i;
        assertEquals(used, type.getMethod("used", int.class, int.class).invoke(instance, 31, limit));
        int wide = (int)0x100000003L + 7 + sum;
        for (int i = 0; i < limit; i++) wide ^= i;
        assertEquals(wide, type.getMethod("wide", long.class, int.class, int.class).invoke(null, 0x100000003L, 7, limit));
        assertEquals(limit <= 0 ? 0 : limit * (limit + 1) / 2, type.getMethod("countdown", int.class).invoke(null, limit));
        assertEquals(limit < 0 ? 0 : limit * limit + limit, type.getMethod("clamp", int.class).invoke(null, limit));
      }
    }
  }
}
