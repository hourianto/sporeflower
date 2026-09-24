package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;

import java.lang.reflect.Array;
import java.lang.reflect.Method;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class ArrayPostIncrementArgumentRegressionTest extends DecompileRegressionTestBase {
  private static final Pattern POSTFIX_ARGUMENT = Pattern.compile("record\\(this\\.values\\[var\\d+ \\+ 3]\\+\\+, this\\.values\\[var\\d+ \\+ 3]\\)");

  @Test
  public void preservesOldByteArgumentAndNarrowedStore() throws Exception {
    Path original = fixture.getTestDataDir().resolve("classes/jasm");
    checkBehavior(original);
    String content = decompileClassFile(original.resolve("pkg/TestArrayPostIncrementArgument.class"), "pkg/TestArrayPostIncrementArgument.java");
    recompile();
    checkBehavior(fixture.getTempDir().resolve("recompiled-out"));

    // Output can have the right value in a quiet array and still duplicate the
    // read. Verify the JVM dup remains one read after round-tripping as well.
    assertEquals(1, countCapturedReads(original.resolve("pkg/TestArrayPostIncrementArgument.class")));
    assertEquals(1, countCapturedReads(fixture.getTempDir().resolve("recompiled-out/pkg/TestArrayPostIncrementArgument.class")));

    // The retained old byte should read as the postfix increment it came from.
    assertTrue(POSTFIX_ARGUMENT.matcher(content).find(), content);
  }

  @Test
  public void keepsOverloadAndIndexEvaluation() throws Exception {
    Path source = writeSource("pkg/TestArrayPostIncrementUses.java", """
      package pkg;
      public class TestArrayPostIncrementUses {
        public static int calls;
        static int pick(byte value) { return 1000 + value; }
        static int pick(int value) { return 2000 + value; }
        static int pick(short value) { return 3000 + value; }
        static int pick(char value) { return 4000 + value; }
        static int index() { calls++; return 1; }
        static int use(int a, int b) { return a * 1000 + b; }

        public static int byteOverload(byte[] values) { return pick(values[1]++); }
        public static int intOverload(byte[] values) { return pick((int)values[1]++); }
        public static int shortOverload(short[] values) { return pick(values[1]--); }
        public static int charOverload(char[] values) { return pick(values[1]++); }
        public static int sideEffectIndex(byte[] values) { return use(values[index()]++, values[1]); }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    checkUses(outRoot());
    decompileDirectory(outRoot(), "pkg/TestArrayPostIncrementUses.java");
    recompile();
    checkUses(fixture.getTempDir().resolve("recompiled-out"));
  }

  @Test
  public void keepsCompoundAssignmentsAfterSavedValue() throws Exception {
    Path source = writeSource("pkg/TestSavedCompoundAssignments.java", """
      package pkg;
      public class TestSavedCompoundAssignments {
        public byte byteField;
        public int intField;

        public static int byteArray(byte[] a) {
          byte old = a[0];
          a[0] += (byte)(old + 1);
          return old * 1000 + a[0];
        }

        public static int intArray(int[] a) {
          int old = a[0];
          a[0] += old + 1;
          return old * 1000 + a[0];
        }

        public int byteFieldUpdate() {
          byte old = byteField;
          byteField += (byte)(old + 1);
          return old * 1000 + byteField;
        }

        public int intFieldUpdate() {
          int old = intField;
          intField -= old - 1;
          return old * 1000 + intField;
        }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    checkCompoundAssignments(outRoot());
    decompileDirectory(outRoot(), "pkg/TestSavedCompoundAssignments.java");
    recompile();
    checkCompoundAssignments(fixture.getTempDir().resolve("recompiled-out"));
  }

  // A compound assignment updates the element or field; it is not the plain store of old + 1 that a postfix encodes.
  private static void checkCompoundAssignments(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("pkg.TestSavedCompoundAssignments");
      for (int seed : new int[]{-128, -1, 5, 127}) {
        String context = classes + ": seed " + seed;
        byte[] bytes = {(byte)seed};
        assertEquals(seed * 1000 + (byte)(seed + (byte)(seed + 1)),
          type.getMethod("byteArray", byte[].class).invoke(null, (Object)bytes), context + " byte[]");
        int[] ints = {seed};
        assertEquals(seed * 1000 + seed + seed + 1,
          type.getMethod("intArray", int[].class).invoke(null, (Object)ints), context + " int[]");

        Object instance = type.getConstructor().newInstance();
        type.getField("byteField").setByte(instance, (byte)seed);
        assertEquals(seed * 1000 + (byte)(seed + (byte)(seed + 1)),
          type.getMethod("byteFieldUpdate").invoke(instance), context + " byte field");
        type.getField("intField").setInt(instance, seed);
        assertEquals(seed * 1000 + 1, type.getMethod("intFieldUpdate").invoke(instance), context + " int field");
      }
    }
  }

  private static void checkUses(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("pkg.TestArrayPostIncrementUses");
      for (int seed : new int[]{-128, -1, 0, 5, 127}) {
        String context = classes + ": seed " + seed;
        byte[] bytes = {0, (byte)seed};
        assertEquals(1000 + seed, type.getMethod("byteOverload", byte[].class).invoke(null, (Object)bytes), context + " pick(byte)");
        assertEquals((byte)(seed + 1), bytes[1], context + " pick(byte) store");
        bytes = new byte[]{0, (byte)seed};
        assertEquals(2000 + seed, type.getMethod("intOverload", byte[].class).invoke(null, (Object)bytes), context + " pick(int)");
        assertEquals((byte)(seed + 1), bytes[1], context + " pick(int) store");

        short[] shorts = {0, (short)(seed * 256)};
        assertEquals(3000 + seed * 256, type.getMethod("shortOverload", short[].class).invoke(null, (Object)shorts), context + " pick(short)");
        assertEquals((short)(seed * 256 - 1), shorts[1], context + " pick(short) store");
        char[] chars = {0, (char)(seed & 0xFFFF)};
        assertEquals(4000 + (seed & 0xFFFF), type.getMethod("charOverload", char[].class).invoke(null, (Object)chars), context + " pick(char)");
        assertEquals((char)((seed & 0xFFFF) + 1), chars[1], context + " pick(char) store");

        type.getField("calls").setInt(null, 0);
        bytes = new byte[]{0, (byte)seed};
        assertEquals(seed * 1000 + (byte)(seed + 1), type.getMethod("sideEffectIndex", byte[].class).invoke(null, (Object)bytes), context + " index");
        assertEquals(1, type.getField("calls").getInt(null), context + " index() calls");
      }
    }
  }

  @ParameterizedTest
  @ValueSource(strings = {"byte", "short", "char", "int", "long", "float", "double"})
  public void preservesPrimitiveUpdatesAndCapturedReads(String primitive) throws Exception {
    Path source = writeSource("pkg/TestArrayUpdates.java", """
      package pkg;
      public class TestArrayUpdates {
        public static %1$s increment(%1$s[] values, int index) { return values[index]++; }
        public static %1$s decrement(%1$s[] values, int index) { return values[index]--; }
        public static %1$s prefixIncrement(%1$s[] values, int index) { return ++values[index]; }
        public static %1$s prefixDecrement(%1$s[] values, int index) { return --values[index]; }
        public static %1$s capture(%1$s[] values, int index) {
          %1$s old = values[index];
          values[index] = (%1$s)(old + 7);
          return old;
        }
      }
      """.formatted(primitive));
    compileJava8NoDebug(source, outRoot());
    checkUpdates(outRoot(), primitive);
    decompileDirectory(outRoot(), "pkg/TestArrayUpdates.java");
    recompile();
    checkUpdates(fixture.getTempDir().resolve("recompiled-out"), primitive);
  }

  private static void checkUpdates(Path classes, String primitive) throws Exception {
    Class<?> component = switch (primitive) {
      case "byte" -> byte.class;
      case "short" -> short.class;
      case "char" -> char.class;
      case "int" -> int.class;
      case "long" -> long.class;
      case "float" -> float.class;
      case "double" -> double.class;
      default -> throw new AssertionError(primitive);
    };
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("pkg.TestArrayUpdates");
      for (String name : new String[]{"increment", "decrement", "prefixIncrement", "prefixDecrement", "capture"}) {
        int delta = name.equals("capture") ? 7 : name.toLowerCase().contains("decrement") ? -1 : 1;
        Object values = Array.newInstance(component, 3);
        Method method = type.getMethod(name, values.getClass(), int.class);
        for (long seed : new long[]{Long.MIN_VALUE, Integer.MIN_VALUE, -32768, -128, -1, 0, 1, 127, 32767, 65535, Integer.MAX_VALUE, Long.MAX_VALUE}) {
          Object old = narrow(primitive, seed, 0);
          Array.set(values, 1, old);
          long integral = old instanceof Character c ? c.charValue() : ((Number)old).longValue();
          Object stored = primitive.equals("float") ? (Object)(((Number)old).floatValue() + delta)
            : primitive.equals("double") ? (Object)(((Number)old).doubleValue() + delta)
            : narrow(primitive, integral, delta);
          String context = classes + ": " + primitive + " " + name + "(" + seed + ")";
          assertEquals(name.startsWith("prefix") ? stored : old, method.invoke(null, values, 1), context + " result");
          assertEquals(stored, Array.get(values, 1), context + " store");
        }
      }
    }
  }

  private static Object narrow(String primitive, long value, int delta) {
    return switch (primitive) {
      case "byte" -> (byte)(value + delta);
      case "short" -> (short)(value + delta);
      case "char" -> (char)(value + delta);
      case "int" -> (int)(value + delta);
      case "long" -> value + delta;
      case "float" -> (float)value + delta;
      case "double" -> (double)value + delta;
      default -> throw new AssertionError(primitive);
    };
  }

  private static int countCapturedReads(Path classFile) throws Exception {
    int[] reads = {0};
    new ClassReader(Files.readAllBytes(classFile)).accept(new ClassVisitor(Opcodes.ASM9) {
      @Override
      public MethodVisitor visitMethod(int access, String name, String descriptor, String signature, String[] exceptions) {
        return !name.equals("twice") ? null : new MethodVisitor(Opcodes.ASM9) {
          @Override
          public void visitInsn(int opcode) {
            if (opcode == Opcodes.BALOAD) reads[0]++;
          }
        };
      }
    }, 0);
    return reads[0];
  }

  private static void checkBehavior(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("pkg.TestArrayPostIncrementArgument");
      for (int seed = Byte.MIN_VALUE; seed <= Byte.MAX_VALUE; seed++) {
        for (boolean enabled : new boolean[]{false, true}) {
          Object instance = type.getConstructor().newInstance();
          byte[] values = new byte[8];
          values[5] = (byte)seed;
          type.getField("values").set(instance, values);
          type.getField("observed").setInt(instance, 1000);
          type.getField("stored").setInt(instance, 1000);
          type.getMethod("draw", int.class, boolean.class).invoke(instance, 2, enabled);
          String context = classes + ": seed=" + seed + ", enabled=" + enabled;
          assertEquals(enabled ? seed : 1000, type.getField("observed").getInt(instance), context + " old argument");
          assertEquals(enabled ? (byte)(seed + 1) : 1000, type.getField("stored").getInt(instance), context + " later argument");
          assertEquals(enabled ? (byte)(seed + 1) : seed, values[5], context + " array store");
        }
      }

      // combine(saved, mutate(values), saved) must pass the element read before mutate() on both sides.
      int[] elements = {4};
      assertEquals(474, type.getMethod("aroundMutation", int[].class).invoke(null, (Object)elements), classes + ": aroundMutation");
      assertEquals(5, elements[0], classes + ": aroundMutation store");
    }
  }
}
