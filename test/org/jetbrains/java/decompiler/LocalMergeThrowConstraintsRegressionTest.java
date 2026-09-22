package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.util.DataInputFullStream;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.MethodSource;

import java.lang.reflect.InvocationTargetException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.stream.Stream;

import static org.junit.jupiter.api.Assertions.*;

class LocalMergeThrowConstraintsRegressionTest extends DecompileRegressionTestBase {
  static Stream<Object[]> cases() {
    return Stream.of("RuntimeException", "Error", "Checked").flatMap(base ->
      Stream.of(false, true).flatMap(strict -> Stream.of(false, true).map(debug -> new Object[]{base, strict, debug})));
  }

  @ParameterizedTest(name = "{0}, strict={1}, debug={2}")
  @MethodSource("cases")
  void coalescingPreservesThrowEffectsAndExceptionIdentity(String base, boolean strict, boolean debug) throws Exception {
    fixture.tearDown();
    fixture.setUp(IFernflowerPreferences.J2ME_STRICT_SLOT_MERGE, strict ? "1" : "0",
      IFernflowerPreferences.BUNDLED_J2ME_API, "0", IFernflowerPreferences.INCLUDE_ENTIRE_CLASSPATH, "0");
    supplyThrowableHierarchy();

    boolean checked = base.equals("Checked");
    String throwsSecond = checked ? " throws ThrownLocals.Second" : "";
    String body = """
      int result = 0;
      { First value = first(); result += use(value); result += use(value); }
      { Second value = second(); result += use(value); if (input == 1) throw value; result += use(value); }
      { Third value = third(); result += use(value); result += use(value); }
      return result;
      """;
    Path input = writeSource("ThrownLocals.java", """
      public class ThrownLocals implements ThrowAction {
        public static class Checked extends Exception { }
        public static class First extends %s { }
        public static class Second extends %s { }
        public static class Third extends %s { }
        public static Throwable last;
        public static int visits;
        static First first() { return new First(); }
        static Second second() { Second value = new Second(); last = value; return value; }
        static Third third() { return new Third(); }
        static int use(Object value) { visits++; return value == null ? 0 : 1; }
        public int run(int input)%s { %s }
        public static int direct(int input)%s { %s }
        public static int caught(int input) {
          try { direct(input); return 9; }
          catch (Second failure) { return failure == last ? 11 : 12; }
        }
        public static int nullThrow(int input)%s {
          int result;
          { First value = first(); result = use(value) + use(value); }
          { %s value = null; result += use(value); if (input == 1) throw value; result += use(value); }
          return result;
        }
      }
      interface ThrowAction { int run(int input)%s; }
      """.formatted(base, base, base, throwsSecond, body, throwsSecond, body,
        checked ? " throws Checked" : "", base, throwsSecond));
    if (debug) compileJava8WithDebug(input, outRoot());
    else compileJava8NoDebug(input, outRoot());
    assertReusedSlot(outRoot().resolve("ThrownLocals.class"));
    assertBehavior(outRoot(), checked);
    String source = decompileDirectory(outRoot(), "ThrownLocals.java");
    recompile();
    assertBehavior(fixture.getTempDir().resolve("recompiled-out"), checked);
    assertFalse(source.contains("catch (Throwable"), "Type selection must not induce exception wrappers:\n" + source);
    assertFalse(source.contains("throws Throwable"), "Type selection must not widen the public throws contract:\n" + source);
    if (!debug && !checked) {
      String run = source.substring(source.indexOf("public int run("), source.indexOf("public static int direct("));
      assertEquals(1, run.split(base + " ", -1).length - 1,
        "Unchecked sibling lifetimes should still share one readable local:\n" + run);
    }
  }

  private void supplyThrowableHierarchy() throws Exception {
    // Without the library hierarchy the common superclass cannot be resolved,
    // so this regression would silently pass by rejecting every sibling merge.
    Path library = fixture.getTempDir().resolve("library");
    for (Class<?> type : List.of(Object.class, Throwable.class, Exception.class, RuntimeException.class, Error.class)) {
      String resource = type.getName().replace('.', '/') + ".class";
      Path target = library.resolve(resource);
      Files.createDirectories(target.getParent());
      try (var stream = type.getResourceAsStream("/" + resource)) {
        assertNotNull(stream);
        Files.copy(stream, target);
      }
    }
    fixture.getDecompiler().addLibrary(library.toFile());
  }

  private static void assertReusedSlot(Path file) throws Exception {
    try (var stream = new DataInputFullStream(Files.readAllBytes(file))) {
      StructClass type = StructClass.create(stream, true);
      for (String name : List.of("run", "direct")) {
        var method = type.getMethod(name, "(I)I");
        method.expandData(type);
        var instructions = method.getInstructionSequence();
        List<Integer> slots = new ArrayList<>();
        for (int i = 0; i < instructions.length() - 1; i++) {
          var instruction = instructions.getInstr(i);
          if (instruction.opcode != CodeConstants.opc_invokestatic) continue;
          var call = type.getPool().getLinkConstant(instruction.operand(0));
          if (!List.of("first", "second", "third").contains(call.elementname)) continue;
          var store = instructions.getInstr(i + 1);
          assertEquals(CodeConstants.opc_astore, store.opcode);
          slots.add(store.operand(0));
        }
        assertEquals(3, slots.size());
        assertEquals(1, slots.stream().distinct().count(), "Fixture must reuse one JVM slot: " + slots);
      }
    }
  }

  private static void assertBehavior(Path classes, boolean checked) throws Exception {
    try (var loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("ThrownLocals");
      Object instance = type.getConstructor().newInstance();
      for (String name : List.of("run", "direct", "nullThrow")) {
        var method = type.getMethod(name, int.class);
        Class<?>[] declared = checked ? new Class<?>[]{loader.loadClass("ThrownLocals$" + (name.equals("nullThrow") ? "Checked" : "Second"))}
          : new Class<?>[0];
        assertArrayEquals(declared, method.getExceptionTypes(), "Throws contract changed in " + name);
        for (int input : new int[]{0, 1, 2}) {
          type.getField("visits").setInt(null, 0);
          if (input == 1) {
            InvocationTargetException failure = assertThrows(InvocationTargetException.class, () -> method.invoke(instance, input), name);
            if (name.equals("nullThrow")) {
              assertInstanceOf(NullPointerException.class, failure.getCause());
            } else {
              assertSame(type.getField("last").get(null), failure.getCause(), "Thrown object changed in " + name);
              assertEquals("ThrownLocals$Second", failure.getCause().getClass().getName());
            }
          } else {
            assertEquals(name.equals("nullThrow") ? 2 : 6, method.invoke(instance, input), name);
          }
          assertEquals(input == 1 ? 3 : name.equals("nullThrow") ? 4 : 6, type.getField("visits").getInt(null), name);
        }
      }
      assertEquals(11, type.getMethod("caught", int.class).invoke(null, 1), "The original specific catch must still handle the throw");
      assertEquals(9, type.getMethod("caught", int.class).invoke(null, 0));
    }
  }
}
