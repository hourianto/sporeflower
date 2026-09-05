package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.CsvSource;

import java.io.IOException;
import java.lang.reflect.InvocationTargetException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

public class ConstructorLivePreludeRegressionTest extends DecompileRegressionTestBase {
  @ParameterizedTest
  @CsvSource({
    "TestConstructorSavedArgumentBeforeSuper, false",
    "TestConstructorSavedArgumentsBeforeSuper, true",
    "TestConstructorRepeatedSavedArgument, false",
    "TestConstructorRepeatedSavedArguments, true"
  })
  public void testLiveArgumentsSurviveSlotReuseBeforeSuper(String name, boolean usesSecond) throws Exception {
    Path input = inputDirectory();
    copyClass("jasm", name, input);
    copyClass("java8", "TestConstructorPreludeBase", input);
    assertSavedArguments(input, name, usesSecond);

    String content = decompileDirectory(input, "pkg/" + name + ".java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    recompile();
    assertSavedArguments(fixture.getTempDir().resolve("recompiled-out"), name, usesSecond);
  }

  @Test
  public void testCursorAdvancePrecedesThisDelegation() throws Exception {
    Path input = inputDirectory();
    copyClass("jasm", "TestConstructorStatefulThisPrelude", input);
    copyClass("java8", "TestConstructorPreludeInput", input);
    assertStatefulDelegation(input);

    String content = decompileDirectory(input, "pkg/TestConstructorStatefulThisPrelude.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    recompile();
    assertStatefulDelegation(fixture.getTempDir().resolve("recompiled-out"));
  }

  @Test
  public void testIndependentEffectsKeepTheirOrderAndExceptionBoundaries() throws Exception {
    Path input = inputDirectory();
    copyClass("jasm", "TestConstructorOrderedPrelude", input);
    copyClass("java8", "TestConstructorPreludeEffects", input);
    assertOrderedEffects(input);

    decompileDirectory(input, "pkg/TestConstructorOrderedPrelude.java");
    recompile();
    assertOrderedEffects(fixture.getTempDir().resolve("recompiled-out"));
  }

  @Test
  public void testUnsafeHelperExtractionLeavesTheConstructorIntact() throws Exception {
    Path input = inputDirectory();
    copyClass("jasm", "TestConstructorUnliftablePrelude", input);
    copyClass("java8", "TestConstructorPreludeEffects", input);
    try (URLClassLoader loader = loader(input)) {
      Class<?> type = loader.loadClass("pkg.TestConstructorUnliftablePrelude");
      assertUnliftableConstructor(type, new Class<?>[0], new Object[0], 1, 2, 7);
      assertUnliftableConstructor(type, new Class<?>[]{boolean.class}, new Object[]{true}, 1, 2, 1);
      assertUnliftableConstructor(type, new Class<?>[]{String.class}, new Object[]{"input"}, 2, 1, 0);
    }

    // These shapes need more than independent static argument helpers. This is
    // a safety test for declining extraction, not a successful source roundtrip.
    String content = decompileDirectory(input, "pkg/TestConstructorUnliftablePrelude.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(content.contains("$sporeflower$preinit$"), content);
  }

  private static void assertUnliftableConstructor(
    Class<?> type, Class<?>[] parameterTypes, Object[] arguments, int first, int second, int saved
  ) throws Exception {
    Class<?> base = type.getSuperclass();
    List<?> events = (List<?>)base.getField("events").get(null);
    events.clear();
    Object instance = type.getConstructor(parameterTypes).newInstance(arguments);
    assertEquals(first, base.getField("first").getInt(instance));
    assertEquals(second, base.getField("second").getInt(instance));
    assertEquals(saved, type.getField("saved").getInt(instance));
    assertEquals(List.of(1, 3, 5), events);
  }

  private Path inputDirectory() throws IOException {
    Path input = fixture.getTempDir().resolve("input");
    Files.createDirectories(input.resolve("pkg"));
    return input;
  }

  private void copyClass(String version, String name, Path input) throws IOException {
    Files.copy(fixture.getTestDataDir().resolve("classes/" + version + "/pkg/" + name + ".class"),
      input.resolve("pkg/" + name + ".class"));
  }

  private static URLClassLoader loader(Path classes) throws IOException {
    return new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader());
  }

  private static void assertOrderedEffects(Path classes) throws Exception {
    try (URLClassLoader loader = loader(classes)) {
      Class<?> type = loader.loadClass("pkg.TestConstructorOrderedPrelude");
      Class<?> base = loader.loadClass("pkg.TestConstructorPreludeEffects");
      var constructor = type.getConstructor();
      List<?> events = (List<?>)base.getField("events").get(null);
      Object instance = constructor.newInstance();
      assertEquals(3, base.getField("first").getInt(instance));
      assertEquals(4, base.getField("second").getInt(instance));
      List<Integer> expected = List.of(0, 1, 2, 3, 4, 5, 6);
      assertEquals(expected, events);
      for (int step : expected) {
        events.clear();
        base.getField("failingStep").setInt(null, step);
        InvocationTargetException failure = assertThrows(InvocationTargetException.class, constructor::newInstance);
        assertInstanceOf(IllegalStateException.class, failure.getCause());
        assertEquals("step " + step, failure.getCause().getMessage());
        assertEquals(expected.subList(0, step + 1), events);
      }
      base.getField("failingStep").setInt(null, -1);
      for (int step : List.of(1, 3)) {
        events.clear();
        base.getField("checkedStep").setInt(null, step);
        InvocationTargetException failure = assertThrows(InvocationTargetException.class, constructor::newInstance);
        assertInstanceOf(IOException.class, failure.getCause());
        assertEquals("checked " + step, failure.getCause().getMessage());
        assertEquals(expected.subList(0, step + 1), events);
      }
    }
  }

  private static void assertSavedArguments(Path classes, String name, boolean usesSecond) throws Exception {
    try (URLClassLoader loader = loader(classes)) {
      Class<?> type = Class.forName("pkg." + name, true, loader);
      Class<?> base = loader.loadClass("pkg.TestConstructorPreludeBase");
      var constructor = type.getConstructor(Object.class, Object.class);
      for (Object[] arguments : List.of(
        new Object[]{new String("first"), new StringBuffer("second")},
        new Object[]{null, null},
        new Object[]{new String("later"), new StringBuffer("different")}
      )) {
        base.getField("calls").setInt(null, 0);
        Object instance = constructor.newInstance(arguments);
        assertEquals(1, base.getField("calls").getInt(null));
        assertSame(instance, type.getField("instance").get(null));
        assertSame(arguments[0], type.getField("first").get(null));
        assertSame(usesSecond ? arguments[1] : null, type.getField("second").get(null));
        assertTrue(type.getField("ready").getBoolean(instance));
      }

      base.getField("calls").setInt(null, 0);
      InvocationTargetException badCast = assertThrows(InvocationTargetException.class,
        () -> constructor.newInstance(new Object(), null));
      assertInstanceOf(ClassCastException.class, badCast.getCause());
      assertEquals(1, base.getField("calls").getInt(null), "The cast belongs after superclass initialization");

      RuntimeException failure = new IllegalStateException("super failed");
      base.getField("failure").set(null, failure);
      base.getField("calls").setInt(null, 0);
      type.getField("instance").set(null, null);
      InvocationTargetException failedSuper = assertThrows(InvocationTargetException.class,
        () -> constructor.newInstance("first", null));
      assertSame(failure, failedSuper.getCause());
      assertEquals(1, base.getField("calls").getInt(null));
      assertNull(type.getField("instance").get(null), "Subclass work must not run after a failed super call");
    }
  }

  private static void assertStatefulDelegation(Path classes) throws Exception {
    try (URLClassLoader loader = loader(classes)) {
      Class<?> inputType = Class.forName("pkg.TestConstructorPreludeInput", true, loader);
      Class<?> type = loader.loadClass("pkg.TestConstructorStatefulThisPrelude");
      var constructor = type.getConstructor(inputType);
      for (String payload : List.of("payload", "", "another value")) {
        String label = "label";
        byte[] data = inputBytes(0x12345678, label, payload);
        Object input = inputType.getConstructor(byte[].class).newInstance((Object)data);
        Object instance = constructor.newInstance(input);
        assertEquals(0x12345678, type.getField("id").getInt(instance));
        assertEquals(label, type.getField("label").get(instance));
        assertEquals(payload, type.getField("payload").get(instance));
        assertEquals(data.length, inputType.getField("position").getInt(input));
        assertEquals(0, inputType.getField("remaining").getInt(input));
        assertEquals(1, inputType.getField("constructions").getInt(null));
        int lengthOffset = 8 + label.length() * 2;
        int start = lengthOffset + 4;
        int end = start + payload.length();
        assertEquals(List.of("int@0", "text@4", "int@4", "int@" + lengthOffset,
          "decode@" + start + ":" + end, "advance@" + start + ":" + payload.length(),
          "construct@" + end, "int@" + end), inputType.getField("events").get(null));
      }

      // A failed decode must not advance the payload cursor or enter the delegated constructor.
      byte[] valid = inputBytes(7, "label", "payload");
      byte[] truncated = Arrays.copyOf(valid, valid.length - 6);
      Object input = inputType.getConstructor(byte[].class).newInstance((Object)truncated);
      InvocationTargetException failure = assertThrows(InvocationTargetException.class, () -> constructor.newInstance(input));
      assertInstanceOf(ArrayIndexOutOfBoundsException.class, failure.getCause());
      assertEquals(0, inputType.getField("constructions").getInt(null));
      assertEquals(22, inputType.getField("position").getInt(input));
      assertEquals(List.of("int@0", "text@4", "int@4", "int@18", "decode@22:29"),
        inputType.getField("events").get(null));
    }
  }

  private static byte[] inputBytes(int id, String label, String payload) {
    ByteBuffer bytes = ByteBuffer.allocate(16 + label.length() * 2 + payload.length()).order(ByteOrder.LITTLE_ENDIAN);
    bytes.putInt(id).putInt(label.length());
    for (int i = 0; i < label.length(); i++) bytes.putChar(label.charAt(i));
    bytes.putInt(payload.length());
    for (int i = 0; i < payload.length(); i++) bytes.put((byte)payload.charAt(i));
    bytes.putInt(0x34567812);
    return bytes.array();
  }
}
